//! The toast queue: what the launcher is currently telling the user.
//!
//! A port of `ToastService` and `Toast`
//! (`src/server/src/services/toast/toast-service.hpp`), minus Qt. The C++ keeps
//! the queue alive with signals and a `QTimer::singleShot` per toast; this
//! keeps the same state machine with an explicit clock, so the behaviour can be
//! tested without a running event loop and driven by whatever the host's own
//! loop is.
//!
//! # The queue that is always one deep
//!
//! `setToast` calls `clear()` before pushing, with the C++'s own comment saying
//! why: "for now we only handle one toast and we replace it every time. We will
//! use the stack if we find the need to handle many toasts concurrently." The
//! queue is kept here because the C++ keeps it, and because the expiry rule
//! only makes sense with it: a timer belongs to *its* toast, so one firing
//! after its toast was replaced closes nothing.

/// How a toast is styled, and whether it expires.
///
/// The C++ `ToastStyle` is a `std::uint8_t` enum in this order; the discriminants
/// are not on any wire, but the order is what a `static_cast` would see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToastStyle {
    /// The default, and what `success` uses.
    #[default]
    Success,
    /// Informational.
    Info,
    /// A warning.
    Warning,
    /// What `failure` uses.
    Danger,
    /// A toast that does **not** expire, and that `dynamic` updates in place.
    Dynamic,
}

/// The C++ `setToast`'s default duration, in milliseconds.
pub const DEFAULT_DURATION_MS: u64 = 2000;

/// One toast.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Toast {
    /// The bold line.
    pub title: String,
    /// The second line; often empty.
    pub message: String,
    /// Its style, which also decides whether it expires.
    pub style: ToastStyle,
}

/// What the view is told to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// `toastActivated`: show this.
    Activated(Toast),
    /// `toastHidden`: show nothing.
    Hidden,
}

/// One toast's pending expiry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Expiry {
    toast: u64,
    at_ms: u64,
}

/// The queue, and the events it has raised but nobody has drained.
#[derive(Debug, Default)]
pub struct ToastService {
    queue: Vec<(u64, Toast)>,
    expiries: Vec<Expiry>,
    events: Vec<Event>,
    next_id: u64,
}

impl ToastService {
    /// An empty service.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The toast that should be on screen, if any. The C++ `currentToast` is
    /// the *back* of the queue, not the front.
    #[must_use]
    pub fn current(&self) -> Option<&Toast> {
        self.queue.last().map(|(_, toast)| toast)
    }

    /// The events raised since the last drain, oldest first.
    pub fn drain_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// `ToastService::clear`.
    ///
    /// Emits `Hidden` unconditionally, as the C++ does — even when there was
    /// nothing to hide.
    /// Dropping the pending expiries here is the one place this differs in
    /// mechanism from the C++, where the `QTimer` outlives the toast, fires at
    /// its own time and finds nothing in the queue to close. The outcome is the
    /// same — a replaced toast's deadline never touches its replacement — and
    /// nothing has to stay armed for a toast that is gone.
    pub fn clear(&mut self) {
        self.queue.clear();
        self.expiries.clear();
        self.events.push(Event::Hidden);
    }

    /// `ToastService::setToast`.
    ///
    /// Replaces whatever is showing, which is why a caller sees `Hidden` and
    /// then `Activated` for a single call.
    pub fn set_toast(
        &mut self,
        title: &str,
        style: ToastStyle,
        message: &str,
        duration_ms: u64,
        now_ms: u64,
    ) {
        self.clear();
        let id = self.next_id;
        self.next_id += 1;

        // `if (priority != ToastStyle::Dynamic)` -- a dynamic toast stays until
        // something replaces it, because it is how a command reports progress.
        if style != ToastStyle::Dynamic {
            self.expiries.push(Expiry {
                toast: id,
                at_ms: now_ms.saturating_add(duration_ms),
            });
        }

        self.queue.push((
            id,
            Toast {
                title: title.to_owned(),
                message: message.to_owned(),
                style,
            },
        ));
        self.update_current();
    }

    /// `ToastService::success`, which is `setToast` with the default duration.
    pub fn success(&mut self, title: &str, message: &str, now_ms: u64) {
        self.set_toast(
            title,
            ToastStyle::Success,
            message,
            DEFAULT_DURATION_MS,
            now_ms,
        );
    }

    /// `ToastService::failure`, which is `Danger` — not an `Error` style.
    pub fn failure(&mut self, title: &str, message: &str, now_ms: u64) {
        self.set_toast(
            title,
            ToastStyle::Danger,
            message,
            DEFAULT_DURATION_MS,
            now_ms,
        );
    }

    /// `ToastService::dynamic`.
    ///
    /// Updates the current toast *in place* when it is already dynamic, rather
    /// than replacing it: a progress toast that was replaced on every update
    /// would flicker, and each replacement would emit `Hidden` first.
    pub fn dynamic(&mut self, title: &str, message: &str, now_ms: u64) {
        if let Some((_, current)) = self.queue.last_mut()
            && current.style == ToastStyle::Dynamic
        {
            current.title = title.to_owned();
            current.message = message.to_owned();
            // `setTitle`/`setMessage` each emit `updated`, which the service
            // answers with `updateCurrent`; one activation per update here,
            // because two identical ones say nothing the first did not.
            self.update_current();
            return;
        }

        self.set_toast(
            title,
            ToastStyle::Dynamic,
            message,
            DEFAULT_DURATION_MS,
            now_ms,
        );
    }

    /// Closes whatever has expired by `now_ms`.
    ///
    /// This is the `QTimer::singleShot(duration, toast.get(), ...)` the C++
    /// arms per toast. The timer holds *its* toast, so one that fires after its
    /// toast has been replaced finds nothing to close and says nothing —
    /// without that, a short toast arriving after a long one would cut the long
    /// one short.
    pub fn tick(&mut self, now_ms: u64) {
        let due: Vec<u64> = self
            .expiries
            .iter()
            .filter(|expiry| expiry.at_ms <= now_ms)
            .map(|expiry| expiry.toast)
            .collect();
        self.expiries.retain(|expiry| expiry.at_ms > now_ms);

        for toast in due {
            if let Some(index) = self.queue.iter().position(|(id, _)| *id == toast) {
                self.queue.remove(index);
                self.update_current();
            }
        }
    }

    /// `ToastService::updateCurrent`.
    fn update_current(&mut self) {
        match self.queue.last() {
            Some((_, toast)) => self.events.push(Event::Activated(toast.clone())),
            None => self.events.push(Event::Hidden),
        }
    }
}
