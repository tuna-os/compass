//! The configuration's global shortcuts, bound on the desktop, and what
//! happens when one fires: `GlobalShortcutService`.
//!
//! What is bound is `compass_core::global_shortcuts`' to decide: the
//! launcher hotkey (`launcher.hotkey`) and every command's
//! `providers.<p>.entrypoints.<e>.shortcut`. This is the part that knows the
//! desktop. A [`Backend`] binds one shortcut at a time and reports each
//! press as the id it was bound under; a [`Service`] keeps the backend in
//! step with the configuration (`reconcile`), suspends every binding while
//! the shortcut recorder captures (`setCapturing`), and turns a press into
//! an [`Action`]: the launcher's toggle, pushed to the attached window, or a
//! command launched as `cmd launch` and root search launch it
//! (`activateEntrypoint`).
//!
//! # Which backend
//!
//! The C++ factory's order, then the portal the C++ does not have:
//!
//! 1. `xx-hotkey-v1`, then `vicinae-hotkey-v1`, over Wayland
//!    ([`compass_wayland::hotkey`]), on any compositor but GNOME's.
//! 2. `org.freedesktop.portal.GlobalShortcuts` ([`ShortcutBinder`]): on
//!    GNOME 50/51, the first target, the only path an unprivileged
//!    application has. The portal binds a set at a time, so the backend
//!    collects the binds of one reconcile and hands the whole set over at
//!    the end of it.
//!
//! No backend is a warning, never a reason to stop: the engine still answers
//! `vicinae toggle`, and on a wlroots compositor the log says how to bind
//! that by hand.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use compass_core::Config;
use compass_core::global_shortcuts::{self, Action, Binding, Reconciler};
use compass_ipc::{Response, WindowCommand};
use compass_portals::{
    PortalConfig, Portals, ShortcutBinder, ShortcutDescriptor, ShortcutEvent, ShortcutsOutcome,
    Trigger,
};
use tokio::sync::{RwLock, mpsc, watch};

use crate::serve::{EngineState, forward};

/// What the rest of the engine uses to steer the service: a new
/// configuration to bind, and the recorder capturing.
#[derive(Debug)]
pub struct Control {
    generation: watch::Sender<u64>,
    capturing: watch::Sender<bool>,
}

impl Default for Control {
    fn default() -> Self {
        Self {
            generation: watch::Sender::new(0),
            capturing: watch::Sender::new(false),
        }
    }
}

impl Control {
    /// The configuration changed: bind what it now says (`configChanged`).
    pub fn reload(&self) {
        self.generation.send_modify(|generation| *generation += 1);
    }

    /// The shortcut recorder started or stopped capturing.
    pub fn set_capturing(&self, capturing: bool) {
        self.capturing.send_if_modified(|current| {
            let changed = *current != capturing;
            *current = capturing;
            changed
        });
    }
}

/// Binds shortcuts on the desktop, one at a time
/// (`AbstractGlobalShortcutBackend`).
pub trait Backend: Send {
    /// Its name, for logs.
    fn name(&self) -> &'static str;

    /// Binds `binding`, or says why the desktop would not.
    fn bind(&mut self, binding: &Binding) -> impl Future<Output = Result<(), String>> + Send;

    /// Releases `id`.
    fn unbind(&mut self, id: &str) -> impl Future<Output = ()> + Send;

    /// Releases everything.
    fn unbind_all(&mut self) -> impl Future<Output = ()> + Send;

    /// Carries out the binds and unbinds since the last flush where the
    /// backend collects them, and names each id that then failed, with why.
    fn flush(&mut self) -> impl Future<Output = Vec<(String, String)>> + Send {
        async { Vec::new() }
    }
}

/// A backend kept in step with the configuration.
#[derive(Debug)]
pub struct Service<B> {
    backend: B,
    reconciler: Reconciler,
    capturing: bool,
}

impl<B: Backend> Service<B> {
    /// Nothing bound yet.
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            reconciler: Reconciler::default(),
            capturing: false,
        }
    }

    /// The backend, for tests.
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// `reconcile`: releases what is no longer wanted and binds what is new.
    /// Nothing while capturing.
    pub async fn reconcile(&mut self, desired: &BTreeMap<String, Binding>) {
        if self.capturing {
            return;
        }
        let plan = self.reconciler.plan(desired);
        if plan.is_empty() {
            return;
        }
        for id in &plan.unbind {
            self.backend.unbind(id).await;
        }
        for binding in &plan.bind {
            let result = self.backend.bind(binding).await;
            if let Err(reason) = &result {
                tracing::warn!(
                    id = %binding.id, trigger = %binding.trigger, %reason,
                    "failed to bind a global shortcut"
                );
            }
            self.reconciler.bound(binding, &result);
        }
        for (id, reason) in self.backend.flush().await {
            if let Some(binding) = desired.get(&id) {
                tracing::warn!(
                    %id, trigger = %binding.trigger, %reason,
                    "failed to bind a global shortcut"
                );
                self.reconciler.bound(binding, &Err(reason));
            }
        }
        tracing::info!(
            backend = self.backend.name(),
            bound = ?self.reconciler.live(),
            "global shortcuts bound"
        );
    }

    /// `setCapturing`: every binding is released while the recorder
    /// captures, so it sees the combination rather than the desktop, and
    /// `desired` is bound again after.
    pub async fn set_capturing(&mut self, capturing: bool, desired: &BTreeMap<String, Binding>) {
        if self.capturing == capturing {
            return;
        }
        self.capturing = capturing;
        if capturing {
            self.backend.unbind_all().await;
            self.backend.flush().await;
            self.reconciler.clear();
        } else {
            self.reconcile(desired).await;
        }
    }

    /// What pressing `id` does (`onActivated`); `None` for an id that is
    /// not bound.
    #[must_use]
    pub fn action(&self, id: &str) -> Option<Action> {
        self.reconciler.action(id).cloned()
    }
}

/// Carries out a pressed shortcut's action.
pub async fn activate(state: &Arc<RwLock<EngineState>>, action: Action) -> Response {
    match action {
        Action::ToggleLauncher => {
            let slot = state.read().await.window_slot();
            forward(&slot, WindowCommand::Toggle, "toggle a window").await
        }
        Action::RunCommand(id) => crate::serve::launch_entrypoint(state, id).await,
    }
}

/// What the configuration asks to be bound, each item named by its title.
pub async fn desired(
    state: &Arc<RwLock<EngineState>>,
    config: &Config,
) -> BTreeMap<String, Binding> {
    let state = state.read().await;
    global_shortcuts::desired(config, |id| {
        state.index().root(id).map(|root| root.title.clone())
    })
}

/// Reads `vicinae.json`, or the defaults when it cannot be read.
async fn load_config() -> Config {
    match tokio::task::spawn_blocking(Config::load).await {
        Ok(Ok(config)) => config,
        Ok(Err(err)) => {
            tracing::warn!(error = %err, "global shortcuts: using the default configuration");
            Config::default()
        }
        Err(_) => Config::default(),
    }
}

/// Runs `service` until its presses stop: binds what `config` reads, again
/// on each reload, suspends while capturing, and acts on each press.
pub async fn serve<B, F, Fut>(
    state: Arc<RwLock<EngineState>>,
    mut service: Service<B>,
    mut presses: mpsc::UnboundedReceiver<String>,
    config: F,
) where
    B: Backend,
    F: Fn() -> Fut,
    Fut: Future<Output = Config>,
{
    let control = state.read().await.global_shortcuts();
    let mut generation = control.generation.subscribe();
    let mut capturing = control.capturing.subscribe();
    service
        .reconcile(&desired(&state, &config().await).await)
        .await;
    loop {
        tokio::select! {
            press = presses.recv() => {
                let Some(id) = press else { break };
                if let Some(action) = service.action(&id) {
                    let state = Arc::clone(&state);
                    tokio::spawn(async move {
                        if let Response::Error(err) = activate(&state, action).await {
                            tracing::warn!(%id, error = %err.message, "a global shortcut did nothing");
                        }
                    });
                }
            }
            changed = generation.changed() => {
                if changed.is_err() {
                    break;
                }
                service.reconcile(&desired(&state, &config().await).await).await;
            }
            changed = capturing.changed() => {
                if changed.is_err() {
                    break;
                }
                let on = *capturing.borrow_and_update();
                let desired = desired(&state, &config().await).await;
                service.set_capturing(on, &desired).await;
            }
        }
    }
    tracing::info!("global shortcuts: the backend went away; they are unbound");
}

/// Binds the configuration's global shortcuts for as long as the engine
/// runs.
pub async fn run(state: Arc<RwLock<EngineState>>) {
    let (presses_tx, presses) = mpsc::unbounded_channel();
    if let Some(backend) = connect_wayland(presses_tx.clone()).await {
        serve(state, Service::new(backend), presses, load_config).await;
        return;
    }
    if let Some(backend) = connect_portal(presses_tx).await {
        serve(state, Service::new(backend), presses, load_config).await;
        return;
    }
    let hint = match crate::wlroots::detect().await {
        Some(wlroots) => compass_wayland::hotkey::manual_binding_hint(wlroots.desktop.as_deref()),
        None => "bind `vicinae toggle` to a key in your desktop's settings".to_owned(),
    };
    tracing::warn!("no global shortcut backend; {hint}. `vicinae doctor` explains the options");
}

/// `xx-hotkey-v1` or `vicinae-hotkey-v1`, on any Wayland compositor but
/// GNOME's (whose path is the portal, whatever Mutter grows).
async fn connect_wayland(presses: mpsc::UnboundedSender<String>) -> Option<WaylandBackend> {
    use compass_wayland::compositor::{current_desktop, desktop_is_gnome};
    use compass_wayland::hotkey::{HotkeyClient, HotkeyError, HotkeyEvent};

    if std::env::var_os("WAYLAND_DISPLAY").is_none()
        || desktop_is_gnome(current_desktop().as_deref())
    {
        return None;
    }
    let (events_tx, mut events) = mpsc::unbounded_channel();
    let client = match tokio::task::spawn_blocking(move || {
        HotkeyClient::connect(compass_ui::APP_ID, events_tx)
    })
    .await
    {
        Ok(Ok(client)) => client,
        Ok(Err(HotkeyError::Unsupported)) => {
            tracing::info!("the compositor has neither hotkey protocol; trying the portal");
            return None;
        }
        Ok(Err(err)) => {
            tracing::warn!(error = %err, "the compositor's hotkey protocol failed");
            return None;
        }
        Err(err) => {
            tracing::warn!(error = %err, "the hotkey task failed");
            return None;
        }
    };
    tracing::info!(
        protocol = client.protocol().name(),
        "global shortcuts over the compositor's hotkey protocol"
    );
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                HotkeyEvent::Triggered { id, .. } => {
                    if presses.send(id).is_err() {
                        break;
                    }
                }
                HotkeyEvent::Revoked { id, message } => tracing::warn!(
                    %id, message,
                    "the compositor revoked a global shortcut"
                ),
            }
        }
    });
    Some(WaylandBackend {
        client,
        bound: BTreeMap::new(),
    })
}

/// The GlobalShortcuts portal, where the session bus has one.
async fn connect_portal(presses: mpsc::UnboundedSender<String>) -> Option<PortalBackend> {
    let portals = match Portals::connect(PortalConfig::default()).await {
        Ok(portals) => portals,
        Err(err) => {
            tracing::warn!(error = %err, "no portal connection for global shortcuts");
            return None;
        }
    };
    let availability = portals.capabilities().global_shortcuts;
    if !availability.is_available() {
        tracing::warn!(%availability, "no GlobalShortcuts portal");
        return None;
    }
    let (events_tx, mut events) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            if let Some(id) = portal_press(event)
                && presses.send(id).is_err()
            {
                break;
            }
        }
    });
    Some(PortalBackend::new(ShortcutBinder::new(portals, events_tx)))
}

/// The id a portal event presses, if it is a press.
///
/// A release is not one: toggling on it too would open the launcher on the
/// press and close it on the release, which looks exactly like a hotkey
/// that does nothing.
pub fn portal_press(event: ShortcutEvent) -> Option<String> {
    match event {
        ShortcutEvent::Activated { id, .. } => Some(id),
        ShortcutEvent::Changed { shortcuts } => {
            for bound in shortcuts {
                tracing::info!(
                    id = %bound.id,
                    trigger = bound.trigger_description,
                    "the desktop changed a shortcut binding"
                );
            }
            None
        }
        _ => None,
    }
}

/// The Wayland hotkey protocols: one hotkey object per shortcut.
#[derive(Debug)]
pub struct WaylandBackend {
    client: compass_wayland::hotkey::HotkeyClient,
    bound: BTreeMap<String, compass_wayland::hotkey::Hotkey>,
}

impl Backend for WaylandBackend {
    fn name(&self) -> &'static str {
        self.client.protocol().name()
    }

    async fn bind(&mut self, binding: &Binding) -> Result<(), String> {
        let Some(keysym) = global_shortcuts::keysym(&binding.combo.key) else {
            return Err("Unsupported trigger key".to_owned());
        };
        let request = compass_wayland::hotkey::HotkeyRequest {
            id: binding.id.clone(),
            description: binding.description.clone(),
            keysym,
            modifiers: global_shortcuts::modifier_mask(binding.combo.modifiers),
        };
        let hotkey = self
            .client
            .bind(&request)
            .await
            .map_err(|err| err.to_string())?;
        self.bound.insert(binding.id.clone(), hotkey);
        Ok(())
    }

    async fn unbind(&mut self, id: &str) {
        self.bound.remove(id);
    }

    async fn unbind_all(&mut self) {
        self.bound.clear();
    }
}

/// The portal: the binds of one reconcile, bound as one set on flush.
#[derive(Debug)]
pub struct PortalBackend {
    binder: ShortcutBinder,
    set: BTreeMap<String, ShortcutDescriptor>,
    dirty: bool,
}

impl PortalBackend {
    /// Nothing bound yet.
    #[must_use]
    pub fn new(binder: ShortcutBinder) -> Self {
        Self {
            binder,
            set: BTreeMap::new(),
            dirty: false,
        }
    }
}

/// The portal's descriptor for `binding`: its id, its title, and the
/// trigger it would like, which the desktop may override.
#[must_use]
pub fn portal_descriptor(binding: &Binding) -> ShortcutDescriptor {
    let descriptor = ShortcutDescriptor::new(binding.id.clone(), binding.description.clone());
    match global_shortcuts::portal_trigger(&binding.combo)
        .and_then(|trigger| Trigger::parse(&trigger).ok())
    {
        Some(trigger) => descriptor.with_trigger(trigger),
        None => descriptor,
    }
}

impl Backend for PortalBackend {
    fn name(&self) -> &'static str {
        "GlobalShortcuts portal"
    }

    async fn bind(&mut self, binding: &Binding) -> Result<(), String> {
        self.set
            .insert(binding.id.clone(), portal_descriptor(binding));
        self.dirty = true;
        Ok(())
    }

    async fn unbind(&mut self, id: &str) {
        self.dirty |= self.set.remove(id).is_some();
    }

    async fn unbind_all(&mut self) {
        self.set.clear();
        self.dirty = true;
    }

    async fn flush(&mut self) -> Vec<(String, String)> {
        if !std::mem::take(&mut self.dirty) {
            return Vec::new();
        }
        let set: Vec<ShortcutDescriptor> = self.set.values().cloned().collect();
        let failed = |reason: &str| {
            set.iter()
                .map(|descriptor| (descriptor.id.clone(), reason.to_owned()))
                .collect()
        };
        match self.binder.apply(&set).await {
            Ok(ShortcutsOutcome::Granted { shortcuts }) => {
                for bound in &shortcuts {
                    tracing::info!(
                        id = %bound.id,
                        trigger = bound.trigger_description,
                        "the desktop bound a global shortcut"
                    );
                }
                set.iter()
                    .filter(|descriptor| !shortcuts.iter().any(|bound| bound.id == descriptor.id))
                    .map(|descriptor| {
                        (
                            descriptor.id.clone(),
                            "the desktop did not bind it".to_owned(),
                        )
                    })
                    .collect()
            }
            Ok(ShortcutsOutcome::Denied) => failed("the desktop's permission dialog was dismissed"),
            Ok(_) => failed("the desktop refused the shortcuts"),
            Err(err) => failed(&err.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;
    use std::time::Duration;

    use compass_core::key_combo::KeyCombo;
    use compass_core::{AppIndex, JsonFrecencyStore, SystemClock};
    use compass_ipc::{ErrorKind, SocketPath};

    /// An engine with no window attached.
    fn engine() -> Arc<RwLock<EngineState>> {
        let clock = Arc::new(SystemClock);
        Arc::new(RwLock::new(EngineState::with_index(
            AppIndex::builder().build(),
            Box::new(JsonFrecencyStore::in_memory(clock)),
            SocketPath::exact("/nonexistent/ipc.sock"),
            10,
        )))
    }

    /// Records what it is asked, and refuses what `refuse` names.
    #[derive(Debug, Default, Clone)]
    struct Fake {
        log: Arc<Mutex<Vec<String>>>,
        refuse: Vec<String>,
    }

    impl Fake {
        fn log(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }

    impl Backend for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }

        async fn bind(&mut self, binding: &Binding) -> Result<(), String> {
            self.log
                .lock()
                .unwrap()
                .push(format!("bind {} {}", binding.id, binding.trigger));
            if self.refuse.contains(&binding.id) {
                Err("taken".to_owned())
            } else {
                Ok(())
            }
        }

        async fn unbind(&mut self, id: &str) {
            self.log.lock().unwrap().push(format!("unbind {id}"));
        }

        async fn unbind_all(&mut self) {
            self.log.lock().unwrap().push("unbind all".to_owned());
        }
    }

    fn config(json: &str) -> Config {
        serde_json::from_str(json).expect("a configuration")
    }

    fn wanted(json: &str) -> BTreeMap<String, Binding> {
        global_shortcuts::desired(&config(json), |_| None)
    }

    #[tokio::test]
    async fn the_launcher_hotkey_and_a_commands_shortcut_are_bound_and_rebound_when_changed() {
        let fake = Fake::default();
        let mut service = Service::new(fake.clone());
        service
            .reconcile(&wanted(
                r#"{"providers": {"clipboard": {"entrypoints": {"history": {"shortcut": "super+shift+V"}}}}}"#,
            ))
            .await;
        assert_eq!(
            fake.log(),
            [
                "bind clipboard:history super+shift+V",
                "bind toggle super+space"
            ]
        );
        assert_eq!(service.action("toggle"), Some(Action::ToggleLauncher));
        assert_eq!(
            service.action("clipboard:history"),
            Some(Action::RunCommand("clipboard:history".to_owned()))
        );

        // The settings view changed the launcher hotkey.
        service
            .reconcile(&wanted(
                r#"{"launcher": {"hotkey": "alt+SPACE"},
                    "providers": {"clipboard": {"entrypoints": {"history": {"shortcut": "super+shift+V"}}}}}"#,
            ))
            .await;
        assert_eq!(
            fake.log()[2..],
            ["unbind toggle", "bind toggle alt+SPACE"],
            "only what changed is touched"
        );
    }

    #[tokio::test]
    async fn a_refused_shortcut_does_nothing_when_pressed() {
        let fake = Fake {
            refuse: vec!["a:b".to_owned()],
            ..Fake::default()
        };
        let mut service = Service::new(fake);
        service
            .reconcile(&wanted(
                r#"{"providers": {"a": {"entrypoints": {"b": {"shortcut": "super+B"}}}}}"#,
            ))
            .await;
        assert_eq!(service.action("a:b"), None);
        assert_eq!(service.action("toggle"), Some(Action::ToggleLauncher));
    }

    #[tokio::test]
    async fn capturing_releases_everything_and_binds_it_again_after() {
        let fake = Fake::default();
        let mut service = Service::new(fake.clone());
        let desired = wanted("{}");
        service.reconcile(&desired).await;
        service.set_capturing(true, &desired).await;
        assert_eq!(service.action("toggle"), None, "the recorder gets the keys");
        service.reconcile(&desired).await;
        service.set_capturing(false, &desired).await;
        assert_eq!(
            fake.log(),
            [
                "bind toggle super+space",
                "unbind all",
                "bind toggle super+space"
            ]
        );
        assert_eq!(service.action("toggle"), Some(Action::ToggleLauncher));
    }

    #[tokio::test]
    async fn pressing_the_launcher_hotkey_reaches_the_window() {
        // With no window attached the engine refuses by name, which is the
        // signature of "this press went to the window".
        let state = engine();
        let response = activate(&state, Action::ToggleLauncher).await;
        assert!(
            matches!(response, Response::Error(ref err) if err.kind == ErrorKind::Unsupported),
            "{response:?}"
        );
    }

    #[tokio::test]
    async fn pressing_a_commands_shortcut_launches_it_as_cmd_launch_does() {
        let state = engine();
        let response = activate(&state, Action::RunCommand("nope:missing".to_owned())).await;
        assert!(
            matches!(response, Response::Error(ref err)
                if err.kind == ErrorKind::BadRequest && err.message.contains("nope:missing")),
            "{response:?}"
        );
    }

    #[tokio::test]
    async fn a_reload_binds_what_the_configuration_now_says() {
        let state = engine();
        let fake = Fake::default();
        let (presses_tx, presses) = mpsc::unbounded_channel();
        let current = Arc::new(Mutex::new(config("{}")));
        let read = {
            let current = Arc::clone(&current);
            move || {
                let config = current.lock().unwrap().clone();
                async move { config }
            }
        };
        let task = tokio::spawn(serve(
            Arc::clone(&state),
            Service::new(fake.clone()),
            presses,
            read,
        ));
        let until = |want: usize| {
            let fake = fake.clone();
            async move {
                for _ in 0..200 {
                    if fake.log().len() >= want {
                        return fake.log();
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                panic!("the service did not bind: {:?}", fake.log());
            }
        };
        assert_eq!(until(1).await, ["bind toggle super+space"]);

        *current.lock().unwrap() = config(r#"{"launcher": {"hotkey": "alt+F1"}}"#);
        state.read().await.global_shortcuts().reload();
        assert_eq!(until(3).await[1..], ["unbind toggle", "bind toggle alt+F1"]);

        state.read().await.global_shortcuts().set_capturing(true);
        assert_eq!(until(4).await[3], "unbind all");

        drop(presses_tx);
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the service ends with its presses")
            .expect("no panic");
    }

    #[tokio::test]
    async fn the_recorders_capture_reaches_the_service_over_ipc() {
        let state = engine();
        let mut capturing = state.read().await.global_shortcuts().capturing.subscribe();
        let answer = crate::serve::handle(
            &state,
            compass_ipc::Request::ShortcutCapture { capturing: true },
        )
        .await;
        assert_eq!(answer, Response::Ack);
        assert!(capturing.has_changed().unwrap());
        assert!(*capturing.borrow_and_update());
        let _ = crate::serve::handle(
            &state,
            compass_ipc::Request::ShortcutCapture { capturing: true },
        )
        .await;
        assert!(!capturing.has_changed().unwrap(), "said once per change");
    }

    #[test]
    fn a_release_is_not_a_press() {
        let released = ShortcutEvent::Deactivated {
            id: "toggle".to_owned(),
            timestamp: Duration::from_secs(1),
        };
        assert_eq!(portal_press(released), None);
        assert_eq!(
            portal_press(ShortcutEvent::Changed { shortcuts: vec![] }),
            None
        );
        let pressed = ShortcutEvent::Activated {
            id: "clipboard:history".to_owned(),
            timestamp: Duration::from_secs(1),
            activation_token: None,
        };
        assert_eq!(portal_press(pressed).as_deref(), Some("clipboard:history"));
    }

    #[test]
    fn the_portal_is_asked_for_the_configured_trigger() {
        let binding = Binding {
            id: "toggle".to_owned(),
            trigger: "super+SPACE".to_owned(),
            combo: KeyCombo::parse("super+SPACE").unwrap(),
            description: global_shortcuts::LAUNCHER_DESCRIPTION.to_owned(),
            action: Action::ToggleLauncher,
        };
        let descriptor = portal_descriptor(&binding);
        assert_eq!(descriptor.id, "toggle");
        assert_eq!(
            descriptor
                .preferred_trigger
                .map(|t| t.to_string())
                .as_deref(),
            Some("LOGO+space")
        );
    }
}
