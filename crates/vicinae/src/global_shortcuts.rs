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
use std::time::Duration;

use compass_core::Config;
use compass_core::global_shortcuts::{self, Action, Binding, Reconciler};
use compass_core::key_combo::KeyCombo;
use compass_ipc::{Response, WindowCommand};
use compass_portals::{
    PortalConfig, Portals, ShortcutBinder, ShortcutDescriptor, ShortcutEvent, ShortcutsOutcome,
    Trigger,
};
use tokio::sync::{RwLock, mpsc, oneshot, watch};

use crate::serve::{EngineState, forward};

/// What the rest of the engine uses to steer the service: a new
/// configuration to bind, and the recorder capturing.
#[derive(Debug)]
pub struct Control {
    generation: watch::Sender<u64>,
    capturing: watch::Sender<bool>,
    frontmost: watch::Sender<Option<String>>,
    probes: std::sync::Mutex<Option<mpsc::UnboundedSender<Probe>>>,
}

/// A combination the recorder captured, and where the backend's answer
/// goes.
type Probe = (KeyCombo, oneshot::Sender<Option<String>>);

/// How long the recorder waits for the backend's answer to a probe before
/// taking the combination.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

impl Default for Control {
    fn default() -> Self {
        Self {
            generation: watch::Sender::new(0),
            capturing: watch::Sender::new(false),
            frontmost: watch::Sender::new(None),
            probes: std::sync::Mutex::new(None),
        }
    }
}

impl Control {
    /// The focused application changed to `app` (its desktop id, when it
    /// was recognised): `AppRuntime::frontmostAppChanged`.
    pub fn set_frontmost(&self, app: Option<String>) {
        self.frontmost.send_if_modified(|current| {
            let changed = *current != app;
            *current = app;
            changed
        });
    }

    /// Whether the running backend would bind `combo`
    /// (`GlobalShortcutService::probeBind`): the desktop's refusal, or
    /// `None` when it would, when no backend runs, or when it does not
    /// answer in time.
    pub async fn probe(&self, combo: KeyCombo) -> Option<String> {
        let sender = self
            .probes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()?;
        let (reply, answer) = oneshot::channel();
        sender.send((combo, reply)).ok()?;
        tokio::time::timeout(PROBE_TIMEOUT, answer)
            .await
            .ok()?
            .ok()
            .flatten()
    }

    fn serve_probes(&self, sender: Option<mpsc::UnboundedSender<Probe>>) {
        *self
            .probes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = sender;
    }

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

    /// Whether the desktop would bind `binding`: binds it and releases it at
    /// once (`probeBind`).
    fn probe(&mut self, binding: &Binding) -> impl Future<Output = Result<(), String>> + Send {
        async move {
            let bound = self.bind(binding).await;
            self.unbind(&binding.id).await;
            let failed = self.flush().await;
            bound?;
            match failed.into_iter().find(|(id, _)| *id == binding.id) {
                Some((_, reason)) => Err(reason),
                None => Ok(()),
            }
        }
    }
}

/// A backend kept in step with the configuration.
#[derive(Debug)]
pub struct Service<B> {
    backend: B,
    reconciler: Reconciler,
    capturing: bool,
    inhibited: bool,
}

impl<B: Backend> Service<B> {
    /// Nothing bound yet.
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            reconciler: Reconciler::default(),
            capturing: false,
            inhibited: false,
        }
    }

    /// Whether the recorder is capturing.
    #[must_use]
    pub fn capturing(&self) -> bool {
        self.capturing
    }

    /// `updateInhibition`: every binding is released while the focused
    /// application is one `global_shortcuts.inhibit_apps` lists, so its keys
    /// reach it, and `desired` is bound again once it is not. While the
    /// recorder captures nothing is bound anyway, and its end binds or not
    /// as this says.
    pub async fn set_inhibited(&mut self, inhibited: bool, desired: &BTreeMap<String, Binding>) {
        if self.inhibited == inhibited {
            return;
        }
        self.inhibited = inhibited;
        tracing::info!(
            inhibited,
            "global shortcuts paused for the focused application"
        );
        if self.capturing {
            return;
        }
        if inhibited {
            self.backend.unbind_all().await;
            self.backend.flush().await;
            self.reconciler.clear();
        } else {
            self.reconcile(desired).await;
        }
    }

    /// `probeBind`: the desktop's refusal of `combo`, or `None` when it
    /// would bind it. Only while the recorder captures, when nothing else is
    /// bound for the probe to collide with.
    pub async fn probe(&mut self, combo: &KeyCombo) -> Option<String> {
        if !self.capturing {
            return None;
        }
        self.backend
            .probe(&global_shortcuts::probe(combo))
            .await
            .err()
    }

    /// The backend, for tests.
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// `reconcile`: releases what is no longer wanted and binds what is new.
    /// Nothing while capturing or paused.
    pub async fn reconcile(&mut self, desired: &BTreeMap<String, Binding>) {
        if self.capturing || self.inhibited {
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
    let mut frontmost = control.frontmost.subscribe();
    let (probes_tx, mut probes) = mpsc::unbounded_channel::<Probe>();
    control.serve_probes(Some(probes_tx));
    let paused = |config: &Config, frontmost: &watch::Receiver<Option<String>>| {
        global_shortcuts::inhibited(
            config.global_shortcuts().inhibit_apps(),
            frontmost.borrow().as_deref(),
        )
    };
    {
        let config = config().await;
        let desired = desired(&state, &config).await;
        service
            .set_inhibited(paused(&config, &frontmost), &desired)
            .await;
        service.reconcile(&desired).await;
    }
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
                let config = config().await;
                let desired = desired(&state, &config).await;
                service.set_inhibited(paused(&config, &frontmost), &desired).await;
                service.reconcile(&desired).await;
            }
            changed = capturing.changed() => {
                if changed.is_err() {
                    break;
                }
                let on = *capturing.borrow_and_update();
                let desired = desired(&state, &config().await).await;
                service.set_capturing(on, &desired).await;
            }
            changed = frontmost.changed() => {
                if changed.is_err() {
                    break;
                }
                frontmost.borrow_and_update();
                let config = config().await;
                let desired = desired(&state, &config).await;
                service.set_inhibited(paused(&config, &frontmost), &desired).await;
            }
            Some((combo, reply)) = probes.recv() => {
                // The recorder says it captures before it probes; take that
                // first if this loop has not yet.
                let on = *capturing.borrow_and_update();
                if on != service.capturing() {
                    let desired = desired(&state, &config().await).await;
                    service.set_capturing(on, &desired).await;
                }
                let _ = reply.send(service.probe(&combo).await);
            }
        }
    }
    control.serve_probes(None);
    tracing::info!("global shortcuts: the backend went away; they are unbound");
}

/// Binds the configuration's global shortcuts for as long as the engine
/// runs.
pub async fn run(state: Arc<RwLock<EngineState>>) {
    tokio::spawn(follow_frontmost(Arc::clone(&state)));
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

/// Tells the service which application is focused, now and each time focus
/// moves, from the window-manager providers ([`crate::frontmost`]).
async fn follow_frontmost(state: Arc<RwLock<EngineState>>) {
    let report = |state: Arc<RwLock<EngineState>>| async move {
        let app = crate::frontmost::frontmost(&state).await.app_id;
        state.read().await.global_shortcuts().set_frontmost(app);
    };
    report(Arc::clone(&state)).await;
    let watched = Arc::clone(&state);
    crate::frontmost::watch(watched, || report(Arc::clone(&state))).await;
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

    /// The portal binds a whole set behind the desktop's own dialog, where
    /// the person picks the trigger, so there is nothing to ask ahead of it:
    /// a probe would open that dialog for a throwaway shortcut.
    async fn probe(&mut self, _binding: &Binding) -> Result<(), String> {
        Ok(())
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
    async fn a_listed_application_in_front_releases_every_shortcut_until_it_leaves() {
        let fake = Fake::default();
        let mut service = Service::new(fake.clone());
        let desired = wanted("{}");
        service.reconcile(&desired).await;
        service.set_inhibited(true, &desired).await;
        assert_eq!(
            service.action("toggle"),
            None,
            "the keys reach the application"
        );
        service.reconcile(&desired).await;
        service.set_inhibited(true, &desired).await;
        service.set_inhibited(false, &desired).await;
        assert_eq!(
            fake.log(),
            [
                "bind toggle super+space",
                "unbind all",
                "bind toggle super+space"
            ],
            "nothing is bound while paused, and it is bound once after"
        );
        assert_eq!(service.action("toggle"), Some(Action::ToggleLauncher));

        // Paused while the recorder captures: its end binds nothing.
        service.set_capturing(true, &desired).await;
        service.set_inhibited(true, &desired).await;
        service.set_capturing(false, &desired).await;
        assert_eq!(fake.log()[3..], ["unbind all"]);
        assert_eq!(service.action("toggle"), None);
    }

    #[tokio::test]
    async fn a_probe_binds_and_releases_the_combination_and_says_why_it_was_refused() {
        let fake = Fake {
            refuse: vec![global_shortcuts::PROBE_ID.to_owned()],
            ..Fake::default()
        };
        let mut service = Service::new(fake.clone());
        let combo = KeyCombo::parse("super+Q").unwrap();
        assert_eq!(
            service.probe(&combo).await,
            None,
            "not capturing: nothing is asked"
        );
        assert!(fake.log().is_empty());

        service.set_capturing(true, &wanted("{}")).await;
        assert_eq!(service.probe(&combo).await.as_deref(), Some("taken"));
        assert_eq!(
            fake.log()[1..],
            ["bind @probe super+Q", "unbind @probe"],
            "bound and released at once"
        );

        let mut taking = Service::new(Fake::default());
        taking.set_capturing(true, &wanted("{}")).await;
        assert_eq!(taking.probe(&combo).await, None);
    }

    /// Starts `serve` over `fake` with `json` as the configuration, and
    /// waits for its first bind.
    async fn serving(
        state: &Arc<RwLock<EngineState>>,
        fake: &Fake,
        json: &str,
    ) -> (mpsc::UnboundedSender<String>, tokio::task::JoinHandle<()>) {
        let (presses_tx, presses) = mpsc::unbounded_channel();
        let json = json.to_owned();
        let task = tokio::spawn(serve(
            Arc::clone(state),
            Service::new(fake.clone()),
            presses,
            move || {
                let config = config(&json);
                async move { config }
            },
        ));
        until(fake, |log| !log.is_empty()).await;
        (presses_tx, task)
    }

    async fn until(fake: &Fake, done: impl Fn(&[String]) -> bool) -> Vec<String> {
        for _ in 0..400 {
            let log = fake.log();
            if done(&log) {
                return log;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("the service never got there: {:?}", fake.log());
    }

    #[tokio::test]
    async fn the_frontmost_application_pauses_the_shortcuts_the_configuration_names() {
        let state = engine();
        let fake = Fake::default();
        let control = state.read().await.global_shortcuts();
        let (presses_tx, task) = serving(
            &state,
            &fake,
            r#"{"global_shortcuts": {"inhibit_apps": ["org.gnome.Boxes.desktop"]}}"#,
        )
        .await;

        control.set_frontmost(Some("firefox.desktop".into()));
        control.set_frontmost(Some("org.gnome.Boxes.desktop".into()));
        until(&fake, |log| log.last().is_some_and(|l| l == "unbind all")).await;
        control.set_frontmost(Some("firefox.desktop".into()));
        let log = until(&fake, |log| log.len() == 3).await;
        assert_eq!(
            log,
            [
                "bind toggle super+space",
                "unbind all",
                "bind toggle super+space"
            ]
        );

        drop(presses_tx);
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the service ends with its presses")
            .expect("no panic");
    }

    #[tokio::test]
    async fn the_recorders_probe_reaches_the_backend_over_ipc() {
        let state = engine();
        let probe = |trigger: &str| {
            crate::serve::handle(
                &state,
                compass_ipc::Request::ProbeShortcut {
                    trigger: trigger.to_owned(),
                },
            )
        };
        assert_eq!(
            probe("super+Q").await,
            Response::ShortcutProbe { refusal: None },
            "no backend: nothing to refuse it"
        );
        assert!(matches!(
            probe("not a key").await,
            Response::Error(ref err) if err.kind == ErrorKind::BadRequest
        ));

        let fake = Fake {
            refuse: vec![global_shortcuts::PROBE_ID.to_owned()],
            ..Fake::default()
        };
        let (presses_tx, task) = serving(&state, &fake, "{}").await;
        let _ = crate::serve::handle(
            &state,
            compass_ipc::Request::ShortcutCapture { capturing: true },
        )
        .await;
        assert_eq!(
            probe("super+Q").await,
            Response::ShortcutProbe {
                refusal: Some("taken".to_owned())
            }
        );
        assert!(fake.log().contains(&"bind @probe super+Q".to_owned()));

        drop(presses_tx);
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the service ends with its presses")
            .expect("no panic");
        assert_eq!(
            probe("super+Q").await,
            Response::ShortcutProbe { refusal: None },
            "the service is gone"
        );
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
