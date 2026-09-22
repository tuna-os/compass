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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct Mock {
        supports_paste: bool,
        supports_focus: bool,
        copy_ok: bool,
        copy_called: Cell<bool>,
        paste_called: Cell<bool>,
        restore_called: Cell<bool>,
    }

    impl Mock {
        fn new(supports_paste: bool) -> Self {
            Self {
                supports_paste,
                supports_focus: false,
                copy_ok: true,
                copy_called: Cell::new(false),
                paste_called: Cell::new(false),
                restore_called: Cell::new(false),
            }
        }
    }

    impl PasteEnvironment for Mock {
        type Content = String;
        fn copy(&self, _: &Self::Content) -> bool {
            self.copy_called.set(true);
            self.copy_ok
        }
        fn supports_paste(&self) -> bool {
            self.supports_paste
        }
        fn supports_focus_handoff_detection(&self) -> bool {
            self.supports_focus
        }
        fn focused_foreign_window(&self) -> Option<Window> {
            None
        }
        fn focused_window(&self) -> Option<Window> {
            None
        }
        fn find_app(&self, _: &str) -> Option<String> {
            None
        }
        fn schedule_execute(&self, _: u64) {}
        fn start_focus_polling(&self) {}
        fn stop_focus_polling(&self) {}
        fn paste_to_app(&self, _: &PasteTarget) -> bool {
            self.paste_called.set(true);
            true
        }
        fn schedule_clipboard_restore(&self) {
            self.restore_called.set(true);
        }
    }

    #[test]
    fn supports_paste_gating() {
        let svc = PasteService::new(Mock::new(true));
        assert!(svc.supports_paste());
        let svc2 = PasteService::new(Mock::new(false));
        assert!(!svc2.supports_paste());
    }

    #[test]
    fn has_pending_paste_after_paste_content() {
        let mock = Mock::new(true);
        let mut svc = PasteService::new(mock);
        assert!(!svc.has_pending_paste());
        assert!(svc.paste_content(&"hello".to_owned()));
        assert!(svc.has_pending_paste());
        // execute clears pending even without focus
        let _ = svc.execute_paste();
        assert!(!svc.has_pending_paste());
    }

    #[test]
    fn copy_happens_even_when_paste_cannot() {
        let mock = Mock {
            supports_paste: false,
            supports_focus: false,
            copy_ok: true,
            copy_called: Cell::new(false),
            paste_called: Cell::new(false),
            restore_called: Cell::new(false),
        };
        let mut svc = PasteService::new(mock);
        assert!(!svc.paste_content(&"hello".to_owned()));
        assert!(
            svc.env().copy_called.get(),
            "copy must happen even when paste cannot"
        );
        assert!(!svc.env().paste_called.get());
        assert!(!svc.has_pending_paste());
    }

    #[test]
    fn blind_paste_schedules_without_polling_when_no_focus_detection() {
        // #6: without focus handoff, paste schedules blind 150ms
        struct BlindMock {
            scheduled: Cell<Option<u64>>,
        }
        impl PasteEnvironment for BlindMock {
            type Content = String;
            fn copy(&self, _: &Self::Content) -> bool {
                true
            }
            fn supports_paste(&self) -> bool {
                true
            }
            fn supports_focus_handoff_detection(&self) -> bool {
                false
            }
            fn focused_foreign_window(&self) -> Option<Window> {
                None
            }
            fn focused_window(&self) -> Option<Window> {
                None
            }
            fn find_app(&self, _: &str) -> Option<String> {
                None
            }
            fn schedule_execute(&self, d: u64) {
                self.scheduled.set(Some(d));
            }
            fn start_focus_polling(&self) {}
            fn stop_focus_polling(&self) {}
            fn paste_to_app(&self, _: &PasteTarget) -> bool {
                true
            }
            fn schedule_clipboard_restore(&self) {}
        }
        let mock = BlindMock {
            scheduled: Cell::new(None),
        };
        let mut svc = PasteService::new(mock);
        assert!(svc.paste_content(&"x".to_owned()));
        assert_eq!(svc.env().scheduled.get(), Some(BLIND_PASTE_DELAY_MS));
        assert!(svc.has_pending_paste());
    }

    #[test]
    fn poll_focus_lands_schedules_post_delay_and_timeout_drops_paste() {
        struct PollMock {
            foreign: Cell<bool>,
            scheduled: Cell<Option<u64>>,
            polling: Cell<bool>,
        }
        impl PasteEnvironment for PollMock {
            type Content = String;
            fn copy(&self, _: &Self::Content) -> bool {
                true
            }
            fn supports_paste(&self) -> bool {
                true
            }
            fn supports_focus_handoff_detection(&self) -> bool {
                true
            }
            fn focused_foreign_window(&self) -> Option<Window> {
                if self.foreign.get() {
                    Some(Window {
                        title: "t".to_owned(),
                        wm_class: "c".to_owned(),
                    })
                } else {
                    None
                }
            }
            fn focused_window(&self) -> Option<Window> {
                None
            }
            fn find_app(&self, _: &str) -> Option<String> {
                None
            }
            fn schedule_execute(&self, d: u64) {
                self.scheduled.set(Some(d));
            }
            fn start_focus_polling(&self) {
                self.polling.set(true);
            }
            fn stop_focus_polling(&self) {
                self.polling.set(false);
            }
            fn paste_to_app(&self, _: &PasteTarget) -> bool {
                true
            }
            fn schedule_clipboard_restore(&self) {}
        }
        // Landed case
        let mock = PollMock {
            foreign: Cell::new(true),
            scheduled: Cell::new(None),
            polling: Cell::new(false),
        };
        let mut svc = PasteService::new(mock);
        assert!(svc.paste_content(&"x".to_owned()));
        svc.poll_focus();
        assert_eq!(svc.env().scheduled.get(), Some(POST_FOCUS_DELAY_MS));
        // Poll again after landed already stopped — no further schedule
        // Timeout case: never lands, poll until max
        let mock2 = PollMock {
            foreign: Cell::new(false),
            scheduled: Cell::new(None),
            polling: Cell::new(false),
        };
        let mut svc2 = PasteService::new(mock2);
        assert!(svc2.paste_content(&"y".to_owned()));
        for _ in 0..FOCUS_POLL_MAX {
            svc2.poll_focus();
            if !svc2.has_pending_paste() {
                break;
            }
        }
        assert!(!svc2.has_pending_paste(), "timeout must drop pending");
        assert_eq!(
            svc2.env().scheduled.get(),
            None,
            "timeout schedules nothing"
        );
    }

    #[test]
    fn execute_paste_clears_pending_and_restores_clipboard() {
        let mock = Mock::new(true);
        let mut svc = PasteService::new(mock);
        assert_eq!(svc.execute_paste(), None);
        assert!(svc.paste_content(&"hello".to_owned()));
        let target = svc.execute_paste();
        assert!(target.is_some());
        assert!(svc.env().restore_called.get());
        assert!(!svc.has_pending_paste());
        // second execute returns None — no double paste
        assert_eq!(svc.execute_paste(), None);
    }
}
