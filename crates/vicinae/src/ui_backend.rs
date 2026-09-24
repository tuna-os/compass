//! Socket adapter for the UI's shared application search/history service.

use std::time::Duration;

use compass_ipc::{Request, SocketPath};
use compass_ui::backend::{
    ApplicationBackend, BackendFuture, ClipboardBackend, ClipboardContent, ClipboardRow,
    ClipboardRowKind, DmenuList, ExtensionStart, ExtensionViewState, FileResults, FileRow,
    ProgramList, ScriptOutputState, Shortcut, ShortcutDraft, Snippet, SnippetDraft, WindowBackend,
    WindowRow,
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

/// Uses the same engine/socket as the resident window link.
#[derive(Debug)]
pub struct DaemonBackend {
    socket: SocketPath,
}

impl DaemonBackend {
    /// Bind the adapter to a resolved socket without connecting yet.
    pub fn new(socket: SocketPath) -> Self {
        Self { socket }
    }
}

impl ApplicationBackend for DaemonBackend {
    fn search(&self, query: String) -> BackendFuture<'_, Vec<String>> {
        Box::pin(async move {
            let hits =
                tokio::time::timeout(REQUEST_TIMEOUT, crate::ipc::query(&self.socket, &query))
                    .await
                    .map_err(|_| "Application search timed out".to_owned())?
                    .map_err(|error| error.to_string())?;
            Ok(hits.into_iter().map(|hit| hit.id).collect())
        })
    }

    fn record_launch(&self, key: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            tokio::time::timeout(
                REQUEST_TIMEOUT,
                crate::ipc::send_ack(&self.socket, Request::RecordLaunch { key }),
            )
            .await
            .map_err(|_| "Launch history update timed out".to_owned())?
            .map_err(|error| error.to_string())
        })
    }

    fn run_power_command(&self, id: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::RunPowerCommand { id }, "The power command")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn run_media_command(&self, id: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::RunMediaCommand { id }, "The media command")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn search_files(&self, query: String) -> BackendFuture<'_, FileResults> {
        Box::pin(async move {
            match self
                .ask(
                    Request::SearchFiles {
                        query,
                        category: None,
                    },
                    "File search",
                )
                .await?
            {
                compass_ipc::Response::Files { heading, files } => Ok(FileResults {
                    heading,
                    files: files
                        .into_iter()
                        .map(|file| FileRow {
                            path: file.path,
                            name: file.name,
                            category: file.category,
                        })
                        .collect(),
                }),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn open_file(&self, path: String, reveal: bool) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::OpenFile { path, reveal }, "Opening the file")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn fetch_dmenu(&self, token: u64) -> BackendFuture<'_, DmenuList> {
        Box::pin(async move {
            match self
                .ask(Request::DmenuFetch { token }, "Fetching the dmenu list")
                .await?
            {
                compass_ipc::Response::DmenuList { spec } => Ok(DmenuList {
                    content: spec.content,
                    navigation_title: spec.navigation_title,
                    section_title: spec.section_title,
                    output_index: spec.output_index,
                    placeholder: spec.placeholder,
                    query: spec.query,
                    no_section: spec.no_section,
                    no_quick_look: spec.no_quick_look,
                }),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn choose_dmenu(&self, token: u64, output: Option<String>) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(
                    Request::DmenuChoose { token, output },
                    "Answering the dmenu list",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn list_programs(&self) -> BackendFuture<'_, ProgramList> {
        Box::pin(async move {
            match self.ask(Request::ListPrograms, "Listing programs").await? {
                compass_ipc::Response::Programs {
                    programs,
                    terminal,
                    default_action,
                } => Ok(ProgramList {
                    programs,
                    terminal,
                    default_action,
                }),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn run_program(&self, argv: Vec<String>, terminal: bool, hold: bool) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(
                    Request::RunProgram {
                        argv,
                        terminal,
                        hold,
                    },
                    "Running the program",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn list_scripts(&self) -> BackendFuture<'_, Vec<compass_core::script_scan::ScriptItem>> {
        Box::pin(async move {
            match self
                .ask(Request::ListScripts, "Listing script commands")
                .await?
            {
                compass_ipc::Response::Scripts { scripts } => {
                    Ok(scripts.into_iter().map(script_item).collect())
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn run_script(&self, id: String, arguments: Vec<String>) -> BackendFuture<'_, Option<u64>> {
        Box::pin(async move {
            match self
                .ask(Request::RunScript { id, arguments }, "Running the script")
                .await?
            {
                compass_ipc::Response::ScriptStarted { session } => Ok(session),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn script_output(&self, session: u64) -> BackendFuture<'_, ScriptOutputState> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ScriptOutput { session },
                    "Reading the script's output",
                )
                .await?
            {
                compass_ipc::Response::ScriptOutput {
                    output,
                    finished,
                    exit_code,
                    elapsed_ms,
                } => Ok(ScriptOutputState {
                    output,
                    finished,
                    exit_code,
                    elapsed_ms,
                }),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn stop_script(&self, session: u64) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::StopScript { session }, "Stopping the script")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn list_snippets(&self) -> BackendFuture<'_, Vec<Snippet>> {
        Box::pin(
            async move { snippets(self.ask(Request::ListSnippets, "Listing snippets").await?) },
        )
    }

    fn save_snippet(&self, snippet: SnippetDraft) -> BackendFuture<'_, Vec<Snippet>> {
        Box::pin(async move {
            let request = Request::SaveSnippet {
                id: snippet.id,
                name: snippet.name,
                text: snippet.text,
                keyword: snippet.keyword,
                word: snippet.word,
                apps: snippet.apps,
            };
            snippets(self.ask(request, "Saving the snippet").await?)
        })
    }

    fn remove_snippet(&self, id: String) -> BackendFuture<'_, Vec<Snippet>> {
        Box::pin(async move {
            snippets(
                self.ask(Request::RemoveSnippet { id }, "Removing the snippet")
                    .await?,
            )
        })
    }

    fn expand_snippet(
        &self,
        id: String,
        arguments: Vec<(String, String)>,
    ) -> BackendFuture<'_, String> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ExpandSnippet { id, arguments },
                    "Expanding the snippet",
                )
                .await?
            {
                compass_ipc::Response::Text { text } => Ok(text),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn paste_snippet(&self, id: String, arguments: Vec<(String, String)>) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(
                    Request::PasteSnippet { id, arguments },
                    "Pasting the snippet",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn list_shortcuts(&self) -> BackendFuture<'_, Vec<Shortcut>> {
        Box::pin(async move {
            shortcuts(
                self.ask(Request::ListShortcuts, "Listing shortcuts")
                    .await?,
            )
        })
    }

    fn save_shortcut(&self, shortcut: ShortcutDraft) -> BackendFuture<'_, Vec<Shortcut>> {
        Box::pin(async move {
            let request = Request::SaveShortcut {
                id: shortcut.id,
                name: shortcut.name,
                icon: shortcut.icon,
                url: shortcut.url,
                app: shortcut.app,
            };
            shortcuts(self.ask(request, "Saving the shortcut").await?)
        })
    }

    fn remove_shortcut(&self, id: String) -> BackendFuture<'_, Vec<Shortcut>> {
        Box::pin(async move {
            shortcuts(
                self.ask(Request::RemoveShortcut { id }, "Removing the shortcut")
                    .await?,
            )
        })
    }

    fn open_shortcut(&self, id: String, arguments: Vec<String>) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(
                    Request::OpenShortcut { id, arguments },
                    "Opening the shortcut",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn expand_shortcut(&self, id: String, arguments: Vec<String>) -> BackendFuture<'_, String> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ExpandShortcut { id, arguments },
                    "Expanding the shortcut",
                )
                .await?
            {
                compass_ipc::Response::Text { text } => Ok(text),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn run_extension_command(
        &self,
        id: String,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> BackendFuture<'_, ExtensionStart> {
        Box::pin(async move {
            let arguments_json =
                arguments.map(|arguments| serde_json::Value::Object(arguments).to_string());
            match self
                .ask(
                    Request::RunExtensionCommand { id, arguments_json },
                    "Running the command",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(ExtensionStart::Ran),
                compass_ipc::Response::ExtensionStarted { session } => {
                    Ok(ExtensionStart::View(session))
                }
                compass_ipc::Response::ExtensionNeedsPreferences { title, fields } => {
                    Ok(ExtensionStart::NeedsPreferences {
                        title,
                        fields: fields.into_iter().map(preference_input).collect(),
                    })
                }
                compass_ipc::Response::ExtensionNeedsArguments { title, fields } => {
                    Ok(ExtensionStart::NeedsArguments {
                        title,
                        fields: fields.into_iter().map(preference_input).collect(),
                    })
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn extension_view(&self, session: u64, after: u64) -> BackendFuture<'_, ExtensionViewState> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ExtensionView { session, after },
                    "Reading the view",
                )
                .await?
            {
                compass_ipc::Response::ExtensionView {
                    version,
                    view_json,
                    problem,
                    ended,
                    depth,
                    alert,
                    toast,
                } => Ok(ExtensionViewState {
                    depth,
                    toast: toast.map(|toast| compass_ui::backend::ExtensionToast {
                        failure: toast.style == compass_ipc::ExtensionToastStyle::Failure,
                        animated: toast.style == compass_ipc::ExtensionToastStyle::Animated,
                        title: toast.title,
                        message: toast.message,
                    }),
                    alert: alert.map(|alert| compass_ui::backend::ExtensionPrompt {
                        title: alert.title,
                        message: alert.message,
                        confirm_text: alert.confirm_text,
                        cancel_text: alert.cancel_text,
                    }),
                    version,
                    view: view_json
                        .map(|json| serde_json::from_str(&json).map(Box::new))
                        .transpose()
                        .map_err(|err| {
                            format!("The engine sent a view this launcher cannot read: {err}")
                        })?,
                    problem,
                    ended,
                }),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn extension_event(
        &self,
        session: u64,
        handler: String,
        args: Vec<serde_json::Value>,
    ) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            let args_json = serde_json::Value::Array(args).to_string();
            match self
                .ask(
                    Request::ExtensionEvent {
                        session,
                        handler,
                        args_json,
                    },
                    "Running the action",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn set_extension_preferences(
        &self,
        id: String,
        values: serde_json::Map<String, serde_json::Value>,
    ) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            let values_json = serde_json::Value::Object(values).to_string();
            match self
                .ask(
                    Request::SetExtensionPreferences { id, values_json },
                    "Saving the preferences",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn extension_alert_answer(&self, session: u64, confirmed: bool) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ExtensionAlertAnswer { session, confirmed },
                    "Answering the extension",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn extension_pop(&self, session: u64) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::ExtensionPop { session }, "Going back")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn close_extension(&self, session: u64) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::CloseExtension { session }, "Closing the view")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn choose_files(
        &self,
        choice: compass_ui::extension_fields::FileChoice,
    ) -> BackendFuture<'_, Vec<String>> {
        Box::pin(async move {
            // The portal, not a dialog of our own: inside a Flatpak it is also
            // what grants the extension the file it names.
            let portals =
                compass_portals::Portals::connect(compass_portals::PortalConfig::default())
                    .await
                    .map_err(|err| format!("The file chooser is not available: {err}"))?;
            let chooser = portals
                .file_chooser()
                .map_err(|err| format!("The file chooser is not available: {err}"))?;
            // A picker that takes directories and not files asks for a
            // directory; the portal cannot offer both in one dialog.
            let request = if choice.directories && !choice.files {
                compass_portals::FileChooserRequest::directory("Choose a folder")
            } else {
                compass_portals::FileChooserRequest::file("Choose a file")
            }
            .multiple(choice.multiple);
            let outcome = chooser
                .open(request)
                .await
                .map_err(|err| format!("The file chooser failed: {err}"))?;
            Ok(outcome
                .paths()
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect())
        })
    }
}

impl DaemonBackend {
    /// One request, with the engine's own sentence as the error on a refusal:
    /// the window shows it to the user, who does not need to be told that it
    /// came over a socket.
    async fn ask(&self, request: Request, what: &str) -> Result<compass_ipc::Response, String> {
        let exchange = async {
            let mut client = compass_ipc::Client::connect(self.socket.as_path())
                .await
                .map_err(|_| "The Compass engine is not running".to_owned())?;
            client
                .request(request)
                .await
                .map_err(|error| format!("Could not reach the engine: {error}"))
        };
        match tokio::time::timeout(REQUEST_TIMEOUT, exchange).await {
            Err(_) => Err(format!("{what} timed out")),
            Ok(Err(message)) => Err(message),
            Ok(Ok(compass_ipc::Response::Error(error))) => Err(sentence(&error.message)),
            Ok(Ok(response)) => Ok(response),
        }
    }
}

/// A script as the launcher holds it, from the wire.
fn script_item(entry: compass_ipc::ScriptEntry) -> compass_core::script_scan::ScriptItem {
    use compass_core::script_command::{
        ArgumentDataOption, ArgumentType, OutputMode, ScriptArgument,
    };
    compass_core::script_scan::ScriptItem {
        id: entry.id,
        title: entry.title,
        subtitle: entry.subtitle,
        keywords: entry.keywords,
        mode: OutputMode::parse(&entry.mode).unwrap_or_default(),
        needs_confirmation: entry.needs_confirmation,
        path: entry.path,
        arguments: entry
            .arguments
            .into_iter()
            .map(|argument| ScriptArgument {
                argument_type: match argument.kind.as_str() {
                    "password" => ArgumentType::Password,
                    "dropdown" => ArgumentType::Dropdown,
                    _ => ArgumentType::Text,
                },
                placeholder: argument.placeholder,
                optional: argument.optional,
                // The engine encodes; the launcher only asks.
                percent_encoded: false,
                data: argument
                    .options
                    .into_iter()
                    .next()
                    .map(|(title, value)| ArgumentDataOption { title, value }),
            })
            .collect(),
    }
}

/// The snippet list in an engine answer.
fn snippets(response: compass_ipc::Response) -> Result<Vec<Snippet>, String> {
    use compass_core::snippet_store::{SnippetData, StoredExpansion};
    match response {
        compass_ipc::Response::Snippets { snippets } => Ok(snippets
            .into_iter()
            .map(|entry| Snippet {
                id: entry.id,
                name: entry.name,
                data: match (entry.text, entry.file) {
                    (Some(text), _) => SnippetData::Text { text },
                    (None, Some(file)) => SnippetData::File { file },
                    (None, None) => SnippetData::default(),
                },
                created_at: entry.created_at,
                updated_at: entry.updated_at,
                expansion: entry.keyword.map(|keyword| StoredExpansion {
                    keyword,
                    apps: entry.apps,
                    word: entry.word,
                }),
            })
            .collect()),
        other => Err(format!("Unexpected answer from the engine: {other:?}")),
    }
}

/// The shortcut list in an engine answer.
fn shortcuts(response: compass_ipc::Response) -> Result<Vec<Shortcut>, String> {
    match response {
        compass_ipc::Response::Shortcuts { shortcuts } => Ok(shortcuts
            .into_iter()
            .map(|entry| Shortcut {
                id: entry.id,
                name: entry.name,
                icon: entry.icon,
                url: entry.url,
                app: entry.app,
                open_count: entry.open_count,
                created_at: entry.created_at,
                updated_at: entry.updated_at,
                last_used_at: entry.last_used_at,
            })
            .collect()),
        other => Err(format!("Unexpected answer from the engine: {other:?}")),
    }
}

/// The engine's message with its first letter capitalised, for display.
fn sentence(message: &str) -> String {
    let mut chars = message.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

impl ClipboardBackend for DaemonBackend {
    fn clipboard_history(&self, query: String, limit: u32) -> BackendFuture<'_, Vec<ClipboardRow>> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ClipboardHistory { query, limit },
                    "Clipboard history",
                )
                .await?
            {
                compass_ipc::Response::ClipboardHistory { entries } => {
                    Ok(entries.into_iter().map(row).collect())
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn clipboard_content(&self, id: String) -> BackendFuture<'_, ClipboardContent> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ClipboardContent { id },
                    "Reading the clipboard entry",
                )
                .await?
            {
                compass_ipc::Response::ClipboardContent { mime_type, data } => {
                    Ok(ClipboardContent { mime_type, data })
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn clipboard_paste(&self, id: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self.ask(Request::ClipboardPaste { id }, "Pasting").await? {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn clipboard_set_pinned(&self, id: String, pinned: bool) -> BackendFuture<'_, ()> {
        let what = if pinned { "Pinning" } else { "Unpinning" };
        Box::pin(async move {
            match self
                .ask(Request::ClipboardSetPinned { id, pinned }, what)
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn clipboard_remove(&self, id: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::ClipboardRemove { id }, "Removing the entry")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }
}

impl WindowBackend for DaemonBackend {
    fn list_windows(&self) -> BackendFuture<'_, Vec<WindowRow>> {
        Box::pin(async move {
            match self.ask(Request::ListWindows, "Listing windows").await? {
                compass_ipc::Response::Windows { windows } => Ok(windows
                    .into_iter()
                    .map(|window| WindowRow {
                        id: window.id,
                        app: window.app_name.unwrap_or_else(|| window.wm_class.clone()),
                        title: window.title,
                        wm_class: window.wm_class,
                        pid: window.pid,
                        can_close: window.can_close,
                    })
                    .collect()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn activate_window(&self, id: u32) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::ActivateWindow { id }, "Switching windows")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn close_window(&self, id: u32) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::CloseWindow { id }, "Closing a window")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }
}

fn row(entry: compass_ipc::ClipboardEntry) -> ClipboardRow {
    ClipboardRow {
        id: entry.id,
        preview: entry.preview,
        kind: match entry.kind {
            compass_ipc::ClipboardKind::Text => ClipboardRowKind::Text,
            compass_ipc::ClipboardKind::Link => ClipboardRowKind::Link,
            compass_ipc::ClipboardKind::Image => ClipboardRowKind::Image,
            compass_ipc::ClipboardKind::File => ClipboardRowKind::File,
            compass_ipc::ClipboardKind::Unknown => ClipboardRowKind::Unknown,
        },
        pinned: entry.pinned,
        url_host: entry.url_host,
    }
}

fn preference_input(field: compass_ipc::PreferenceField) -> compass_ui::backend::PreferenceInput {
    use compass_ipc::PreferenceFieldKind;
    use compass_ui::backend::PreferenceInputKind;
    compass_ui::backend::PreferenceInput {
        kind: match field.kind {
            PreferenceFieldKind::Text => PreferenceInputKind::Text,
            PreferenceFieldKind::Password => PreferenceInputKind::Password,
            PreferenceFieldKind::Checkbox { label } => PreferenceInputKind::Checkbox { label },
            PreferenceFieldKind::Dropdown { options } => PreferenceInputKind::Dropdown { options },
            PreferenceFieldKind::Unsupported { declared } => {
                PreferenceInputKind::Unsupported { declared }
            }
        },
        value: field
            .value_json
            .and_then(|json| serde_json::from_str(&json).ok()),
        name: field.name,
        title: field.title,
        description: field.description,
        placeholder: field.placeholder,
        required: field.required,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_engine_refusal_reads_as_a_sentence() {
        assert_eq!(
            sentence("clipboard history is unavailable: no keyring"),
            "Clipboard history is unavailable: no keyring"
        );
        assert_eq!(sentence(""), "");
    }
}
