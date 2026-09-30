//! Snippet keyword expansion: the engine's half of it.
//!
//! `SnippetService`'s `handleKeywordTrigger`, `handleUndo` and
//! `syncServerState` (`src/server/src/services/snippet/snippet-service.hpp`).
//! The input server ([`crate::input_server`]) reports a typed keyword; this
//! finds the snippet, checks the focused application against its app list,
//! expands it, puts the text on the clipboard, and asks the helper to erase
//! the keyword and paste — then puts the old clipboard back.
//!
//! The decisions are `compass_core::input_server::expansion`; what is here
//! is the machine: the focused window, the clipboard, the helper.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use compass_core::input_server::expansion::{self, Settings, UndoRecord};
use compass_core::input_server::wire::{Call, ExpansionMode};
use compass_core::snippet_store::{SerializedSnippet, SnippetData};
use compass_worker_host::clipboard_service::{Clipboard, Content, CopyOptions};
use tokio::sync::RwLock;

use crate::input_server::{InputServer, Notice};
use crate::serve::EngineState;

/// How long after the paste the previous clipboard comes back:
/// `CLIPBOARD_RESTORE_DELAY_MS`.
pub const CLIPBOARD_RESTORE_DELAY: Duration = Duration::from_millis(800);

/// The expander: the helper, and what the engine has told it.
#[derive(Debug)]
pub struct Expander {
    server: InputServer,
    /// The keywords the running helper watches, with their modes.
    registered: Mutex<BTreeMap<String, ExpansionMode>>,
    /// The last expansion, for a Backspace to undo.
    undo: Mutex<Option<UndoRecord>>,
    /// The layout and key delay the running helper has.
    pushed: Mutex<(Option<String>, u32)>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The keywords `snippets` asks to be watched for.
#[must_use]
pub fn keywords(snippets: &[SerializedSnippet]) -> BTreeMap<String, ExpansionMode> {
    snippets
        .iter()
        .filter_map(|snippet| {
            let stored = snippet.expansion.as_ref()?;
            Some((stored.keyword.clone(), expansion::mode(stored)))
        })
        .collect()
}

impl Expander {
    /// The helper.
    #[must_use]
    pub fn server(&self) -> &InputServer {
        &self.server
    }

    /// What `compass input-server status` and doctor report.
    #[must_use]
    pub fn status(&self) -> compass_ipc::InputServerStatus {
        let status = self.server.status();
        compass_ipc::InputServerStatus {
            enabled: self.server.enabled(),
            running: status.running,
            injection: status.injection,
            keywords: u32::try_from(lock(&self.registered).len()).unwrap_or(u32::MAX),
            helper: status.helper.map(|path| path.display().to_string()),
            problem: status.problem,
        }
    }

    /// Brings the helper's keywords in line with `wanted`: what was removed
    /// or changed mode is unregistered, what is new is registered. Nothing is
    /// sent while the helper is down; it gets everything when it is ready.
    pub async fn sync(&self, wanted: BTreeMap<String, ExpansionMode>) {
        if !self.server.status().running {
            return;
        }
        let (remove, add) = {
            let registered = lock(&self.registered);
            let remove: Vec<String> = registered
                .iter()
                .filter(|(keyword, mode)| wanted.get(*keyword) != Some(mode))
                .map(|(keyword, _)| keyword.clone())
                .collect();
            let add: Vec<(String, ExpansionMode)> = wanted
                .iter()
                .filter(|(keyword, mode)| registered.get(*keyword) != Some(mode))
                .map(|(keyword, mode)| (keyword.clone(), *mode))
                .collect();
            (remove, add)
        };
        for keyword in remove {
            lock(&self.registered).remove(&keyword);
            self.server
                .call(Call::RemoveSnippet { trigger: keyword })
                .await;
        }
        for (keyword, mode) in add {
            lock(&self.registered).insert(keyword.clone(), mode);
            self.server
                .call(Call::CreateSnippet {
                    trigger: keyword,
                    mode,
                })
                .await;
        }
    }

    /// `syncServerState`: a fresh helper knows nothing.
    async fn ready(&self, wanted: BTreeMap<String, ExpansionMode>, settings: &Settings) {
        lock(&self.registered).clear();
        *lock(&self.pushed) = (None, expansion::DEFAULT_KEY_DELAY_US);
        self.sync(wanted).await;
        self.apply(settings).await;
    }

    /// Sends the layout and key delay when they differ from what the helper
    /// has.
    async fn apply(&self, settings: &Settings) {
        let (layout, delay) = lock(&self.pushed).clone();
        if settings.layout != layout {
            if let Some(keymap) = settings.keymap() {
                self.server.call(Call::SetKeymap(keymap)).await;
            }
            lock(&self.pushed).0.clone_from(&settings.layout);
        }
        if settings.key_delay_us != delay {
            self.server
                .call(Call::SetKeyDelay(
                    i32::try_from(settings.key_delay_us).unwrap_or(i32::MAX),
                ))
                .await;
            lock(&self.pushed).1 = settings.key_delay_us;
        }
    }

    /// Focus moved: forget the half-typed keyword and the undo.
    pub async fn focus_changed(&self) {
        *lock(&self.undo) = None;
        if self.server.status().running {
            self.server.call(Call::ResetContext).await;
        }
    }
}

/// The Snippets extension's preferences, read fresh from `compass.json`.
async fn settings() -> Settings {
    tokio::task::spawn_blocking(|| {
        let config = compass_core::Config::load().unwrap_or_default();
        Settings::from_preferences(config.provider_preferences(expansion::PROVIDER_ID))
    })
    .await
    .unwrap_or_default()
}

/// The snippets' keywords, from the engine's store.
async fn wanted(state: &Arc<RwLock<EngineState>>) -> BTreeMap<String, ExpansionMode> {
    state
        .read()
        .await
        .snippet_store()
        .map(|store| keywords(store.snippets()))
        .unwrap_or_default()
}

/// Starts the helper as `input_server.enabled` says and handles what it
/// reports, for the life of the engine.
pub async fn run(state: Arc<RwLock<EngineState>>, enabled: bool) {
    let (server, mut notices) = InputServer::start(crate::input_server::find_helper());
    let expander = Arc::new(Expander {
        server,
        registered: Mutex::default(),
        undo: Mutex::default(),
        pushed: Mutex::new((None, expansion::DEFAULT_KEY_DELAY_US)),
    });
    state.write().await.set_expander(Arc::clone(&expander));
    expander.server.set_enabled(enabled);
    tokio::spawn(watch_focus(Arc::clone(&state), Arc::clone(&expander)));

    while let Some(notice) = notices.recv().await {
        match notice {
            Notice::Ready => {
                let settings = settings().await;
                expander.ready(wanted(&state).await, &settings).await;
                tracing::info!(
                    keywords = lock(&expander.registered).len(),
                    "the input server is ready"
                );
            }
            Notice::Trigger(keyword) => expand(&state, &expander, &keyword).await,
            Notice::Undo(keyword) => {
                let settings = settings().await;
                let request = {
                    let mut record = lock(&expander.undo);
                    let request = expansion::undo(record.as_ref(), &keyword, &settings);
                    if request.is_some() {
                        *record = None;
                    }
                    request
                };
                if let Some(request) = request {
                    expander.server.call(Call::InjectUndo(request)).await;
                }
            }
        }
    }
}

/// `handleKeywordTrigger`.
async fn expand(state: &Arc<RwLock<EngineState>>, expander: &Expander, keyword: &str) {
    let settings = settings().await;
    expander.apply(&settings).await;
    if !settings.enabled {
        return;
    }

    let snippet = {
        let state = state.read().await;
        state
            .snippet_store()
            .and_then(|store| store.find_by_keyword(keyword))
            .cloned()
    };
    let Some(snippet) = snippet else { return };
    let Some(stored) = snippet.expansion.clone() else {
        return;
    };
    let SnippetData::Text { text } = &snippet.data else {
        return;
    };

    let frontmost = crate::frontmost::frontmost(state).await;
    if !expansion::allowed(&stored.apps, &frontmost) {
        return;
    }
    tracing::info!(
        keyword,
        app = frontmost.app_id.as_deref().unwrap_or("<unknown>"),
        terminal = frontmost.terminal,
        "snippet expansion"
    );

    let clipboard = clipboard(state).await;
    let previous = match &clipboard {
        Some(clipboard) => {
            let clipboard = Arc::clone(clipboard);
            tokio::task::spawn_blocking(move || clipboard.read().text)
                .await
                .ok()
        }
        None => None,
    };
    let placeholder =
        crate::snippets::needs_clipboard(text).then(|| previous.clone().unwrap_or_default());
    let expanded = crate::snippets::expand(text, &[], placeholder).await;
    let plan = expansion::plan(
        keyword,
        stored.word,
        &expanded,
        frontmost.terminal,
        &settings,
    );

    let Some(clipboard) = clipboard else {
        tracing::warn!("no clipboard to paste the snippet through; it was not expanded");
        return;
    };
    {
        let clipboard = Arc::clone(&clipboard);
        let text = plan.text.clone();
        let _ = tokio::task::spawn_blocking(move || {
            clipboard.copy(Content::Text(text), CopyOptions { concealed: true });
        })
        .await;
    }
    *lock(&expander.undo) = plan.undo.clone();
    expander.server.call(Call::InjectExpand(plan.request)).await;

    // The C++ puts back the selection it last saw; this puts back its text.
    if let Some(previous) = previous.filter(|previous| !previous.is_empty()) {
        tokio::spawn(async move {
            tokio::time::sleep(CLIPBOARD_RESTORE_DELAY).await;
            let _ = tokio::task::spawn_blocking(move || {
                clipboard.copy(Content::Text(previous), CopyOptions { concealed: true });
            })
            .await;
        });
    }
}

/// The clipboard: the Shell extension's, or data-control on wlroots.
async fn clipboard(
    state: &Arc<RwLock<EngineState>>,
) -> Option<Arc<crate::extension_runner::ShellClipboard>> {
    let shell = state.read().await.shell_client();
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        crate::extension_runner::ShellClipboard::available(shell, Some(handle)).map(Arc::new)
    })
    .await
    .ok()
    .flatten()
}

/// Resets the helper's context whenever the focused window changes, as the
/// C++ does on `WindowManager::focusChanged`: from the Shell extension's
/// window signal on GNOME, the toplevel list on wlroots.
async fn watch_focus(state: Arc<RwLock<EngineState>>, expander: Arc<Expander>) {
    crate::frontmost::watch(state, || {
        let expander = Arc::clone(&expander);
        async move { expander.focus_changed().await }
    })
    .await;
}
