//! Pasting into whatever the person was using before the launcher opened.
//!
//! A port of `PasteService` (`src/server/src/services/paste/`), minus the
//! timers and the platform backend.
//!
//! # The hard part is not pasting, it is waiting
//!
//! The launcher has focus. Pasting means putting content on the clipboard,
//! giving focus back, waiting until it has actually landed somewhere, and only
//! then synthesising the keystroke. Every one of those steps can be done at
//! the wrong moment: paste too early and the keystroke goes to a window that
//! is closing, wait too long and the person has moved on.
//!
//! So this is a state machine over a clock it does not own. The environment is
//! asked to schedule a callback and to poll for focus; the tests drive both by
//! hand, which is the only way to test "it waited 30ms" without waiting 30ms.

/// How often focus is polled, in milliseconds.
pub const FOCUS_POLL_INTERVAL_MS: u64 = 5;

/// How many polls before giving up — `FOCUS_POLL_MAX`, five seconds' worth.
pub const FOCUS_POLL_MAX: u32 = 1000;

/// How long to wait after focus lands before pasting, in milliseconds.
///
/// Focus arriving is not the same as the window being ready to take a
/// keystroke, and this gap is what covers the difference.
pub const POST_FOCUS_DELAY_MS: u64 = 30;

/// How long to wait before pasting when focus cannot be observed at all.
///
/// Six times [`POST_FOCUS_DELAY_MS`], because without a signal the only
/// options are to guess long or to lose the paste.
pub const BLIND_PASTE_DELAY_MS: u64 = 150;

/// The whole timeout, for a reader who would rather see it in milliseconds.
#[must_use]
pub const fn focus_timeout_ms() -> u64 {
    FOCUS_POLL_MAX as u64 * FOCUS_POLL_INTERVAL_MS
}

/// The window a paste is aimed at.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Window {
    /// What the window calls itself.
    pub title: String,
    /// The class the app database is looked up by.
    pub wm_class: String,
}

/// What the paste ended up doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasteTarget {
    /// The focused window, if there was one. The C++ pastes anyway when there
    /// is not: "Pasting to unknown window".
    pub window: Option<Window>,
    /// The application that window belongs to, if the database knows it.
    pub app: Option<String>,
}

/// Everything the paste service needs from outside itself.
pub trait PasteEnvironment {
    /// Whatever is being put on the clipboard.
    type Content;

    /// `ClipboardService::copyContent`.
    fn copy(&self, content: &Self::Content) -> bool;

    /// Whether this platform can synthesise a paste at all.
    fn supports_paste(&self) -> bool;

    /// Whether the window manager can tell us focus has moved away.
    fn supports_focus_handoff_detection(&self) -> bool;

    /// `WindowManager::focusedForeignWindow` — focus that is not ours.
    fn focused_foreign_window(&self) -> Option<Window>;

    /// `WindowManager::getFocusedWindow` — whatever has focus now.
    fn focused_window(&self) -> Option<Window>;

    /// `AppService::find`, by window class.
    fn find_app(&self, wm_class: &str) -> Option<String>;

    /// Ask for [`PasteService::execute_paste`] to be called in `delay_ms`.
    fn schedule_execute(&self, delay_ms: u64);

    /// Start polling focus every [`FOCUS_POLL_INTERVAL_MS`].
    fn start_focus_polling(&self);

    /// Stop polling.
    fn stop_focus_polling(&self);

    /// `AbstractPasteService::pasteToApp`.
    fn paste_to_app(&self, target: &PasteTarget) -> bool;

    /// `ClipboardService::scheduleClipboardRestore`.
    fn schedule_clipboard_restore(&self);
}

/// Copy, hand focus back, wait, paste.
#[derive(Debug)]
pub struct PasteService<E> {
    /// The world outside.
    env: E,
    /// Whether a paste is owed.
    pending: bool,
    /// How many times focus has been polled for the current paste.
    poll_count: u32,
}

impl<E: PasteEnvironment> PasteService<E> {
    /// A service over `env`, with nothing pending.
    pub const fn new(env: E) -> Self {
        Self {
            env,
            pending: false,
            poll_count: 0,
        }
    }

    /// The environment, for a caller that has to read back what happened.
    pub const fn env(&self) -> &E {
        &self.env
    }

    /// Whether this platform can paste.
    pub fn supports_paste(&self) -> bool {
        self.env.supports_paste()
    }

    /// Whether a paste is waiting to happen.
    #[must_use]
    pub const fn has_pending_paste(&self) -> bool {
        self.pending
    }

    /// Put `content` on the clipboard and arrange for it to be pasted.
    ///
    /// The `true` means *scheduled*, not *pasted* — the keystroke happens
    /// later, and may still be dropped if focus never lands.
    ///
    /// # The copy happens even when the paste cannot
    ///
    /// The C++ copies first and only then asks whether the platform can paste,
    /// so on a platform that cannot, the content is on the clipboard and the
    /// caller is told `false`. That is arguably the more useful outcome — the
    /// person can paste it themselves — and it is reproduced rather than
    /// tidied, because a caller that retried on `false` would otherwise copy
    /// twice.
    pub fn paste_content(&mut self, content: &E::Content) -> bool {
        if !self.env.copy(content) {
            return false;
        }

        if !self.env.supports_paste() {
            return false;
        }

        // Cancel any in-flight paste: the newest content is the one wanted.
        self.env.stop_focus_polling();
        self.pending = true;

        if self.env.supports_focus_handoff_detection() {
            self.poll_count = 0;
            self.env.start_focus_polling();
        } else {
            self.env.schedule_execute(BLIND_PASTE_DELAY_MS);
        }

        true
    }

    /// One tick of the focus poll timer.
    ///
    /// Counts first and then decides, so the very first tick is poll number
    /// one — with `FOCUS_POLL_MAX` at 1000 that is the difference between five
    /// seconds and five seconds plus a tick, but it is also what makes a
    /// `FOCUS_POLL_MAX` of 1 mean "try once".
    pub fn poll_focus(&mut self) {
        self.poll_count += 1;

        let landed = self.env.focused_foreign_window().is_some();

        if !landed && self.poll_count < FOCUS_POLL_MAX {
            return;
        }

        self.env.stop_focus_polling();

        if landed {
            self.env.schedule_execute(POST_FOCUS_DELAY_MS);
        } else {
            // Timed out. The paste is dropped — but the content stays on the
            // clipboard, so the person can still paste it themselves.
            self.pending = false;
        }
    }

    /// Do the paste, if one is still owed.
    ///
    /// Clears the pending flag *before* pasting, so a second callback — a
    /// blind-paste timer that fired late, say — does not paste twice.
    pub fn execute_paste(&mut self) -> Option<PasteTarget> {
        if !self.pending {
            return None;
        }
        self.pending = false;

        let window = self.env.focused_window();
        let app = window
            .as_ref()
            .and_then(|window| self.env.find_app(&window.wm_class));

        let target = PasteTarget { window, app };
        self.env.paste_to_app(&target);
        // Restored whether or not the paste worked, and whether or not there
        // was a window: the clipboard was borrowed either way.
        self.env.schedule_clipboard_restore();
        Some(target)
    }
}
