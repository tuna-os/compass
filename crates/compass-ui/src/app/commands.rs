//! Opening a builtin command from the root search, and the questions some
//! of them ask first.
//!
//! `open_command` is the one place a `BuiltinCommand` becomes a view, a
//! task or a question: each command's own module opens its view, and this
//! records the use and picks which. A power command asks before it runs
//! when its preference says so; an irreversible action elsewhere asks
//! through a `Confirm`, which Enter answers here.

use super::{ConfirmAction, LauncherApp, Message, Page, Task, focus_search};

impl LauncherApp {
    pub(super) fn open_command(
        &mut self,
        command: &'static compass_core::commands::BuiltinCommand,
    ) -> Task<Message> {
        use compass_core::commands::CommandKind;
        self.panel = None;
        let record = match self.backend.clone() {
            Some(backend) => {
                let key = command.id();
                Task::future(async move {
                    if let Err(error) = backend.record_launch(key).await {
                        tracing::warn!(%error, "could not record opening a command");
                    }
                })
                .discard()
            }
            None => Task::none(),
        };
        match command.kind {
            CommandKind::ClipboardHistory => Task::batch([record, self.open_clipboard_history()]),
            CommandKind::Power(id) => {
                let Some(power) = compass_core::power_commands::command(id) else {
                    return record;
                };
                let asks = self
                    .power_asks
                    .get(power.id)
                    .copied()
                    .unwrap_or(power.confirm_by_default);
                if asks {
                    self.power_confirm = Some(power);
                    return record;
                }
                Task::batch([record, self.run_power_command(power)])
            }
            CommandKind::Media(id) => Task::batch([record, self.run_media(command, id, None)]),
            CommandKind::NowPlaying => Task::batch([record, self.open_now_playing()]),
            CommandKind::ScriptPermissions => Task::batch([record, self.open_script_grants()]),
            CommandKind::SearchTray => Task::batch([record, self.open_search_tray()]),
            CommandKind::OpenSettings => Task::batch([record, self.open_settings(None)]),
            CommandKind::Compass(id) => Task::batch([record, self.open_compass_command(id)]),
            CommandKind::CalculatorHistory => Task::batch([record, self.open_calculator_history()]),
            CommandKind::RefreshExchangeRates => {
                Task::batch([record, self.refresh_exchange_rates(command)])
            }
            CommandKind::BrowseApps => Task::batch([record, self.open_browse_apps()]),
            CommandKind::SetDefaultBrowser => Task::batch([
                record,
                self.open_default_picker(crate::backend::DefaultApp::Browser),
            ]),
            CommandKind::SetDefaultTerminal => Task::batch([
                record,
                self.open_default_picker(crate::backend::DefaultApp::Terminal),
            ]),
            CommandKind::SearchEmojis => Task::batch([record, self.open_emoji_picker()]),
            CommandKind::SearchFiles => {
                Task::batch([record, self.open_search_files(String::new())])
            }
            CommandKind::CreateShortcut => Task::batch([
                record,
                self.open_shortcut_form(compass_core::shortcut_form::Mode::Create, None, false),
            ]),
            CommandKind::ManageShortcuts => Task::batch([record, self.open_manage_shortcuts()]),
            CommandKind::CreateSnippet => Task::batch([
                record,
                self.open_snippet_form(compass_core::shortcut_form::Mode::Create),
            ]),
            CommandKind::ManageSnippets => Task::batch([record, self.open_manage_snippets()]),
            CommandKind::RunProgram => Task::batch([record, self.open_run_program()]),
            CommandKind::SetTheme => Task::batch([record, self.open_set_theme()]),
            CommandKind::CreateExtension => Task::batch([record, self.open_create_extension()]),
            CommandKind::ExtensionStore => {
                self.parked_store = None;
                Task::batch([
                    record,
                    self.open_store_or_intro(crate::backend::Store::Vicinae),
                ])
            }
            CommandKind::RaycastStore => {
                self.parked_store = None;
                Task::batch([
                    record,
                    self.open_store_or_intro(crate::backend::Store::Raycast),
                ])
            }
            CommandKind::BrowseFonts => {
                self.parked_fonts = None;
                Task::batch([record, self.open_browse_fonts()])
            }
            CommandKind::SwitchWindows => {
                self.page = Page::Windows(crate::windows_page::WindowsPage::default());
                Task::batch([record, self.list_windows_task(), focus_search()])
            }
            CommandKind::SwitchWorkspaces => Task::batch([record, self.open_switch_workspaces()]),
            CommandKind::ToggleFullscreen => Task::batch([
                record,
                self.run_window_toggle(crate::backend::WindowToggle::Fullscreen),
            ]),
            CommandKind::ToggleFloating => Task::batch([
                record,
                self.run_window_toggle(crate::backend::WindowToggle::Floating),
            ]),
            CommandKind::ToggleOverview => Task::batch([
                record,
                self.run_window_toggle(crate::backend::WindowToggle::Overview),
            ]),
        }
    }

    /// Hides the launcher, then runs `power` in the engine, as the C++ plan
    /// does (the window closes first). A refusal is shown when it comes back.
    pub(super) fn run_power_command(
        &mut self,
        power: &'static compass_core::power_commands::PowerCommand,
    ) -> Task<Message> {
        self.power_confirm = None;
        let Some(backend) = self.backend.clone() else {
            self.error = Some(format!(
                "{} needs the Compass engine, and this window is running without one",
                power.name
            ));
            return Task::none();
        };
        let id = power.id.to_owned();
        let run = Task::perform(
            async move { backend.run_power_command(id).await },
            Message::BuiltinCommandDone,
        );
        Task::batch([self.conceal(), run])
    }

    /// Runs what the open question asked about, once Enter confirms it.
    pub(super) fn run_confirmed(&mut self) -> Task<Message> {
        let Some(confirm) = self.confirm.take() else {
            return Task::none();
        };
        let task = match confirm.action {
            ConfirmAction::ClipboardRemoveAll => self.remove_all_clipboard_entries(),
            ConfirmAction::RootEdit(id, edit) => self.edit_root_item(id, edit),
            ConfirmAction::UninstallExtension(id) => self.uninstall_extension(id),
            ConfirmAction::RemoveTokenSet(extension, provider) => {
                self.remove_token_set(extension, provider)
            }
        };
        Task::batch([task, focus_search()])
    }
}
