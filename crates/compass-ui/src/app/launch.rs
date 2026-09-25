//! Launches an extension asked for, and the subtitles extensions set.
//!
//! An extension's `launchCommand` and `openCommandPreferences` reach the
//! launcher as a token (`UiCommand::Launch`): the launch is fetched from the
//! engine and taken as if the command had been picked in root search — its
//! arguments form, its preferences form and its view all follow from there.
//! The view that asked is closed first: the launcher shows one extension
//! view at a time, where the C++ pushes the new one above it.

use super::{LauncherApp, Message, Page, Task, focus_search};

impl LauncherApp {
    /// Fetches the launch under `token`.
    pub(super) fn start_launch(&mut self, token: u64) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { backend.fetch_launch(token).await },
            Message::LaunchFetched,
        )
    }

    /// Asks the engine for the subtitles extensions set.
    pub(super) fn refresh_subtitles_task(&self) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { backend.extension_subtitles().await },
            Message::ExtensionSubtitlesLoaded,
        )
    }

    /// Handles [`Message::LaunchFetched`], [`Message::ExtensionSubtitlesLoaded`]
    /// and [`Message::PreferencesOpened`].
    pub(super) fn launch_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::LaunchFetched(Ok(launch)) => {
                let close = self.close_extension_view();
                self.panel = None;
                self.page = Page::Root;
                let Some(backend) = self.backend.clone() else {
                    return close;
                };
                if launch.preferences {
                    let id = launch.id;
                    let opened = id.clone();
                    return Task::batch([
                        close,
                        Task::perform(
                            async move { backend.extension_preferences(id).await },
                            move |result| Message::PreferencesOpened {
                                id: opened.clone(),
                                result,
                            },
                        ),
                    ]);
                }
                let run = match self
                    .app_index
                    .extensions()
                    .iter()
                    .position(|command| command.id == launch.id)
                {
                    Some(index) => self.run_extension_command_with(index, launch.arguments),
                    // The engine knows a command this window's index does
                    // not yet: run it by id, titled by it until its view
                    // names itself.
                    None => {
                        let (id, title) = (launch.id.clone(), launch.id);
                        let arguments = launch.arguments;
                        let started = id.clone();
                        Task::perform(
                            async move { backend.run_extension_command(id, arguments).await },
                            move |result| Message::ExtensionCommandStarted {
                                id: started.clone(),
                                title: title.clone(),
                                result,
                            },
                        )
                    }
                };
                Task::batch([close, run])
            }
            Message::LaunchFetched(Err(reason)) => {
                self.error = Some(reason);
                Task::none()
            }
            Message::ExtensionSubtitlesLoaded(Ok(subtitles)) => {
                self.extension_subtitles = subtitles.into_iter().collect();
                Task::none()
            }
            Message::ExtensionSubtitlesLoaded(Err(reason)) => {
                tracing::debug!(%reason, "no extension subtitles");
                Task::none()
            }
            Message::PreferencesOpened { id, result } => match result {
                Ok(crate::backend::ExtensionStart::NeedsPreferences { title, fields }) => {
                    self.page =
                        Page::Preferences(Box::new(crate::preferences_page::PreferencesPage::new(
                            crate::preferences_page::Purpose::CommandPreferences,
                            id,
                            title,
                            fields,
                        )));
                    focus_search()
                }
                Ok(other) => {
                    tracing::debug!(?other, "a preferences request answered something else");
                    Task::none()
                }
                Err(reason) => {
                    self.error = Some(reason);
                    Task::none()
                }
            },
            _ => Task::none(),
        }
    }
}
