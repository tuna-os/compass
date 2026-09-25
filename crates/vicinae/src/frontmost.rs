//! The focused window and its application, from the window-manager
//! providers (`WindowManager::getFocusedWindow`, `AppRuntime::frontmostApp`):
//! the toplevel list on wlroots, KWin's tracker on Plasma, the Shell
//! extension on GNOME.
//!
//! Snippet expansion asks which application a keyword was typed in, and the
//! global shortcuts ask whether the focused one pauses them
//! (`global_shortcuts.inhibit_apps`); both follow focus with [`watch`].

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use compass_core::input_server::expansion::Frontmost;
use compass_platform_linux::compositor::Provider;
use tokio::sync::RwLock;

use crate::serve::EngineState;

/// How often KWin's tracker, which keeps its windows in memory and signals
/// nothing, is looked at again.
const KWIN_POLL: Duration = Duration::from_millis(500);

/// The focused window's application, recognised in the app index.
pub async fn frontmost(state: &Arc<RwLock<EngineState>>) -> Frontmost {
    let Some(class) = focused_class(state).await else {
        return Frontmost::default();
    };
    let state = state.read().await;
    let apps = compass_core::app_service::AppService::new(state.app_index());
    let Some(app) = apps.find_by_class(&class) else {
        return Frontmost::default();
    };
    Frontmost {
        app_id: Some(app.desktop_id().to_owned()),
        terminal: app.terminal() || app.categories().iter().any(|c| c == "TerminalEmulator"),
    }
}

/// The KWin provider, when this is Plasma: the one provider not reached
/// through the wlroots toplevel list.
fn kwin() -> Option<&'static Provider> {
    crate::wlroots::compositor().filter(|provider| matches!(provider, Provider::Kwin(_)))
}

/// The focused window's class (`WM_CLASS` or Wayland `app_id`) and a handle
/// that changes when focus moves.
pub async fn focused_window(state: &Arc<RwLock<EngineState>>) -> Option<(String, String)> {
    if let Some(wlroots) = crate::wlroots::detect().await {
        let toplevels = wlroots.toplevels.clone()?;
        return toplevels
            .list()
            .into_iter()
            .find(|window| window.activated)
            .map(|window| (window.id.to_string(), window.app_id));
    }
    if let Some(Provider::Kwin(kwin)) = kwin() {
        return kwin
            .focused_window()
            .map(|window| (window.id, window.wm_class));
    }
    let shell = state.read().await.shell_client()?;
    let windows = shell.list_windows().await.ok()?;
    windows
        .into_iter()
        .find(|window| window.focused)
        .map(|window| (format!("{:?}", window.id), window.wm_class))
}

async fn focused_class(state: &Arc<RwLock<EngineState>>) -> Option<String> {
    focused_window(state)
        .await
        .map(|(_, class)| class)
        .filter(|class| !class.is_empty())
}

/// Calls `moved` each time the focused window changes, as the C++ reacts to
/// `WindowManager::focusChanged`: on the toplevel list's changes on
/// wlroots, KWin's tracker looked at twice a second, and the Shell
/// extension's window signal on GNOME. Returns when there is nothing to
/// follow.
pub async fn watch<F, Fut>(state: Arc<RwLock<EngineState>>, mut moved: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ()>,
{
    let mut last = focused_window(&state).await.map(|(id, _)| id);
    let mut changed = |now: Option<String>| {
        let moved = now != last;
        last = now;
        moved
    };

    if let Some(wlroots) = crate::wlroots::detect().await {
        let Some(toplevels) = wlroots.toplevels.clone() else {
            return;
        };
        let mut changes = toplevels.changes();
        while changes.changed().await.is_ok() {
            if changed(focused_window(&state).await.map(|(id, _)| id)) {
                moved().await;
            }
        }
        return;
    }

    // KWin's tracker and the Shell client both arrive after start-up; wait
    // a while for either.
    for _ in 0..START_ATTEMPTS {
        if kwin().is_some() {
            loop {
                tokio::time::sleep(KWIN_POLL).await;
                if changed(focused_window(&state).await.map(|(id, _)| id)) {
                    moved().await;
                }
            }
        }
        let shell = state.read().await.shell_client();
        if let Some(shell) = shell
            && let Ok(mut signals) = shell.windows_changed().await
        {
            while signals.next().await.is_some() {
                if changed(focused_window(&state).await.map(|(id, _)| id)) {
                    moved().await;
                }
            }
            return;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    tracing::info!("no window manager reports focus changes; they are not followed");
}

/// How many seconds [`watch`] waits for KWin's tracker or the Shell
/// extension to appear.
const START_ATTEMPTS: u32 = 30;
