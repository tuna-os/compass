//! The launcher's side of the global shortcuts: hiding on focus loss
//! (`launcher.close_on_focus_loss`, `NavigationController::setWindowActivated`),
//! and telling the engine when the shortcut recorder captures, so the
//! desktop's bindings step aside (`GlobalShortcutBridge::setCapturing`).

use iced::Task;

use super::{LauncherApp, Page};
use crate::message::Message;

impl LauncherApp {
    /// The window gained or lost the focus. Losing it after having had it
    /// hides the window when `close_on_focus_loss` is on, as the C++ closes
    /// on the activated-to-deactivated edge; losing it to the file chooser
    /// the window opened is not the user's doing and is ignored.
    pub(super) fn window_focus_changed(&mut self, focused: bool) -> Task<Message> {
        if focused {
            self.window_focused = true;
            return Task::none();
        }
        let had = std::mem::take(&mut self.window_focused);
        if !had
            || !self.close_on_focus_loss
            || self.choosing_files
            || !self.is_visible()
            || self.closing
        {
            return Task::none();
        }
        self.conceal()
    }

    /// Whether the window is hiding on focus loss, for tests.
    #[must_use]
    pub fn closes_on_focus_loss(&self) -> bool {
        self.close_on_focus_loss
    }

    /// Whether a shortcut recorder is capturing: the action panel's, or the
    /// settings view's.
    pub(super) fn recording_shortcut(&self) -> bool {
        self.panel
            .as_ref()
            .is_some_and(|panel| panel.recorder.is_some())
            || matches!(&self.page, Page::Settings(page) if page.recorder.is_some())
    }

    /// Tells the engine when capturing starts or stops, once per change;
    /// `None` when there is nothing to tell.
    pub(super) fn report_shortcut_capture(&mut self) -> Option<Task<Message>> {
        let capturing = self.recording_shortcut();
        if capturing == self.capture_reported {
            return None;
        }
        self.capture_reported = capturing;
        let backend = self.backend.clone()?;
        Some(Task::perform(
            async move { backend.set_shortcut_capture(capturing).await },
            Message::ShortcutCaptureSet,
        ))
    }

    /// Whether the engine was last told the recorder is capturing, for
    /// tests.
    #[must_use]
    pub fn capture_reported(&self) -> bool {
        self.capture_reported
    }

    /// Every root item's id, title and stored shortcut, for the recorder's
    /// conflict check.
    pub(super) fn recorder_bound(&self) -> Vec<(String, String, String)> {
        self.app_index
            .roots()
            .iter()
            .filter_map(|root| {
                let shortcut = root.meta.shortcut.clone()?;
                Some((root.id.clone(), root.title.clone(), shortcut))
            })
            .collect()
    }
}
