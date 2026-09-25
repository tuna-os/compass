//! Search Files' action panel, as `FileActions::actionPanel` arranges it:
//! Open, Run executable (an AppImage), Show in file browser, Open with…,
//! Set as wallpaper (an image, where the desktop allows), Create shortcut;
//! then Paste, Copy file, Copy file path, Copy file name and Copy mime type.

use compass_core::shortcut_form::Mode;

use super::{LauncherApp, Message, Page, PanelSection, PanelState, Task, focus_search};
use crate::action_panel::Action;
use crate::backend::FileActions;

const OPEN: &str = "file.open";
const RUN: &str = "file.run";
const REVEAL: &str = "file.reveal";
const OPEN_WITH: &str = "file.open-with";
const WALLPAPER: &str = "file.wallpaper";
const CREATE_SHORTCUT: &str = "file.create-shortcut";
const PASTE: &str = "file.paste";
const COPY_FILE: &str = "file.copy";
const COPY_PATH: &str = "file.copy-path";
const COPY_NAME: &str = "file.copy-name";
const COPY_MIME: &str = "file.copy-mime";

/// The panel over the file at `path`, given what it depends on.
#[must_use]
pub fn file_panel_sections(path: &str, info: &FileActions) -> Vec<PanelSection> {
    let mut main = Vec::new();
    if info.has_opener {
        main.push(Action::new("Open").with_id(OPEN).with_shortcut("enter"));
    }
    if crate::files_page::runs_as_executable(path) {
        let run = Action::new("Run executable").with_id(RUN);
        main.push(if info.has_opener {
            run
        } else {
            run.with_shortcut("enter")
        });
    }
    main.push(
        Action::new("Show in file browser")
            .with_id(REVEAL)
            .with_shortcut("ctrl+enter"),
    );
    main.push(
        Action::new("Open with...")
            .with_id(OPEN_WITH)
            .with_shortcut("ctrl+o"),
    );
    if info.can_set_wallpaper
        && info
            .mime
            .as_deref()
            .is_some_and(|mime| mime.starts_with("image/"))
    {
        main.push(
            Action::new("Set as wallpaper")
                .with_id(WALLPAPER)
                .with_shortcut("ctrl+shift+w"),
        );
    }
    main.push(Action::new("Create shortcut").with_id(CREATE_SHORTCUT));
    let mut utils = Vec::new();
    if info.can_paste {
        utils.push(
            Action::new("Paste to active window")
                .with_id(PASTE)
                .with_shortcut("ctrl+shift+v"),
        );
    }
    utils.push(
        Action::new("Copy file")
            .with_id(COPY_FILE)
            .with_shortcut("ctrl+shift+c"),
    );
    utils.push(Action::new("Copy file path").with_id(COPY_PATH));
    utils.push(Action::new("Copy file name").with_id(COPY_NAME));
    if info.mime.is_some() {
        utils.push(Action::new("Copy mime type").with_id(COPY_MIME));
    }
    vec![
        PanelSection {
            name: String::new(),
            actions: main,
        },
        PanelSection {
            name: String::new(),
            actions: utils,
        },
    ]
}

impl LauncherApp {
    /// Asks what the selected file's panel depends on; the panel opens when
    /// the answer comes.
    pub(super) fn open_files_panel(&mut self) -> Option<Task<Message>> {
        let Page::Files(page) = &self.page else {
            return None;
        };
        let path = page.selected_row()?.path.clone();
        let Some(backend) = self.backend.clone() else {
            return Some(self.update(Message::FileActionsLoaded {
                path,
                result: Ok(FileActions::default()),
            }));
        };
        Some(Task::perform(
            async move {
                let result = backend.file_actions(path.clone()).await;
                (path, result)
            },
            |(path, result)| Message::FileActionsLoaded { path, result },
        ))
    }

    fn selected_file(&self) -> Option<(String, String)> {
        let Page::Files(page) = &self.page else {
            return None;
        };
        let row = page.selected_row()?;
        Some((row.path.clone(), row.name.clone()))
    }

    /// Runs a Search Files panel action, if `id` is one.
    pub(super) fn file_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        if !id.starts_with("file.") {
            return None;
        }
        let (path, name) = self.selected_file()?;
        self.panel = None;
        let backend = self.backend.clone();
        let hides = |task: Task<Message>| Task::batch([task, focus_search()]);
        let copy = |this: &mut Self, text: String| {
            Task::batch([iced::clipboard::write(text), this.conceal()])
        };
        Some(match id {
            OPEN => self.open_selected_file(false),
            REVEAL => self.open_selected_file(true),
            OPEN_WITH => self.open_with(path.clone(), path),
            CREATE_SHORTCUT => {
                let task = self.open_shortcut_form(Mode::Create, None, false);
                if let Page::Preferences(form) = &mut self.page {
                    crate::shortcuts_page::prefill(form, &name, &path);
                }
                task
            }
            COPY_PATH => copy(self, path),
            COPY_NAME => copy(self, name),
            COPY_MIME => match self.file_mime.take() {
                Some(mime) => copy(self, mime),
                None => Task::none(),
            },
            RUN | WALLPAPER | PASTE | COPY_FILE => {
                let Some(backend) = backend else {
                    return Some(Task::none());
                };
                let action = id.to_owned();
                hides(Task::perform(
                    async move {
                        match action.as_str() {
                            RUN => backend.run_executable(path, true).await,
                            WALLPAPER => backend.set_wallpaper(path).await,
                            PASTE => backend.copy_file(path, true).await,
                            _ => backend.copy_file(path, false).await,
                        }
                    },
                    Message::FileActionDone,
                ))
            }
            _ => return None,
        })
    }

    /// Handles the file action messages.
    pub(super) fn file_actions_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::FileActionsLoaded { path, result } => {
                if self
                    .selected_file()
                    .is_none_or(|(selected, _)| selected != path)
                {
                    return Task::none();
                }
                let info = result.unwrap_or_else(|error| {
                    tracing::debug!(%error, "no file actions from the engine");
                    FileActions::default()
                });
                self.panel = Some(PanelState::new(file_panel_sections(&path, &info)));
                self.file_mime = info.mime;
                iced::widget::operation::focus(super::PANEL_INPUT)
            }
            Message::FileActionDone(Ok(())) => self.conceal(),
            Message::FileActionDone(Err(reason)) => {
                self.say(reason);
                Task::none()
            }
            _ => Task::none(),
        }
    }
}
