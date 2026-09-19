//! The launcher window: search, move, launch, dismiss.
//!
//! The state machine is deliberately separable from Iced. `update` takes a
//! [`Message`] and mutates fields; the only Iced-shaped things it returns are
//! [`Task`]s, which can be constructed without a running runtime. That is what
//! makes this crate testable in a container with no display server — the
//! selection and launch-target logic, which is where the bugs live, is exercised
//! by ordinary unit tests, and only the drawing needs a compositor.

use iced::{
    Alignment, Border, Color, Element, Length, Padding, Task, Theme,
    widget::{Space, column, container, row, stack, text, text_input},
    window,
};

use std::sync::Arc;

use compass_core::{AppIndex, AppItem};
use compass_platform::{AppLauncher, NullLauncher};
use compass_search::rank_indices;

use crate::action_panel::{self, Action, PanelSection, Row, RowKind, Step};
use crate::design::{self, Appearance, GEOMETRY};
use crate::message::{Direction, Message};
use crate::resident::{EngineLink, UiCommand, UiOutcome};

/// The search field's widget id.
///
/// It exists so the field can be FOCUSED, and that is not a detail. An Iced
/// `text_input` receives typed characters only while it holds widget focus, and
/// nothing focuses it on its own: a launcher whose field is never focused opens,
/// draws a search box, and silently ignores every keystroke until the user
/// thinks to click it. That was #91, and it survived this long because a person
/// trying the launcher clicks the box without noticing they did, while the VM
/// tier -- which only types -- recorded a frame byte-identical to the one before
/// the keystroke and had no way to say why.
///
/// One constant rather than a literal at each site, because the id in `view` and
/// the id passed to `focus` have to be the same string and nothing would report
/// a typo: focus would simply find no widget, and the launcher would go back to
/// ignoring the keyboard. Kept as a `&'static str` rather than an
/// `iced::advanced::widget::Id` so this crate does not have to turn on Iced's
/// `advanced` feature for one constant; both call sites take `impl Into<Id>`.
const SEARCH_INPUT: &str = "compass-search-input";

/// Focus the search field.
///
/// Every path that puts the window on screen ends in one of these, because
/// focus does not survive the window being closed and reopened -- `conceal`
/// closes it, so a summon is a brand new window with a brand new, unfocused
/// field.
fn focus_search() -> Task<Message> {
    iced::widget::operation::focus(SEARCH_INPUT)
}

/// Lifts keyboard events out of the runtime's event stream.
///
/// A free function rather than a closure because [`iced::event::listen_with`]
/// takes a plain `fn` pointer. See [`LauncherApp::subscription`] for why the
/// stream is read at this level rather than through `iced::keyboard::listen`.
fn keyboard_events(
    event: iced::Event,
    _status: iced::event::Status,
    _window: window::Id,
) -> Option<Message> {
    match event {
        iced::Event::Keyboard(event) => Some(Message::Keyboard(event)),
        _ => None,
    }
}

/// Flags for configuring the launcher app.
#[derive(Debug, Clone)]
pub struct AppFlags {
    /// Window configuration.
    pub window_config: window::Settings,
    /// How to launch the selected application.
    ///
    /// Injected rather than reached for: this crate must not know whether it
    /// is on Linux. `vicinae` supplies `compass-platform-linux`'s launcher;
    /// tests supply their own. See ADR-0013.
    pub launcher: Arc<dyn AppLauncher>,
    /// The navigation chord scheme, from `launcher.keybinding`.
    pub keybinding: compass_core::keybinding::Scheme,
    /// Whether the selection wraps, from `launcher.wrap_navigation`.
    pub wrap_navigation: bool,
    /// What the window draws before the desktop says otherwise.
    ///
    /// Separate from `appearance_link` because a window has to draw before any
    /// change can arrive: this is the first frame's colour, and the link only
    /// carries what comes after.
    pub appearance: Appearance,
    /// Changes to the desktop's light/dark preference, when something is
    /// feeding them.
    ///
    /// `None` is a desktop with no Settings portal, or a test. The window then
    /// stays on `appearance` for its whole life, which is the honest outcome:
    /// nothing is telling it otherwise.
    pub appearance_link: Option<crate::appearance::AppearanceLink>,
    /// The engine driving this window, when there is one.
    ///
    /// `None` is the standalone case -- `vicinae ui` run by hand with no daemon
    /// -- and it changes what dismissing means: with nothing able to summon the
    /// window back, hiding it would strand the process invisible, so it exits.
    pub link: Option<EngineLink>,
}

impl Default for AppFlags {
    fn default() -> Self {
        Self {
            window_config: window::Settings {
                size: iced::Size::new(
                    f32::from(GEOMETRY.card_width),
                    f32::from(GEOMETRY.card_max_height),
                ),
                position: window::Position::Centered,
                resizable: false,
                decorations: false,
                transparent: true,
                ..Default::default()
            },
            keybinding: compass_core::keybinding::Scheme::default(),
            wrap_navigation: compass_core::config::DEFAULT_WRAP_NAVIGATION,
            // Deliberately the launcher that launches nothing. A default that
            // silently picked a real backend would make the platform choice
            // invisible at the call site, which is the arrangement ADR-0013
            // exists to end. `vicinae` sets this explicitly.
            launcher: Arc::new(NullLauncher),
            link: None,
            // Dark, until a desktop says otherwise. Not a preference: it is
            // what the launcher has always drawn, so a machine with no
            // Settings portal keeps the appearance it had rather than
            // switching the day this landed.
            appearance: Appearance::Dark,
            appearance_link: None,
        }
    }
}

/// The action panel's own state while it is open.
///
/// Its selection is an `isize` because -1 means "nothing selectable", which is
/// a real state for a panel filtered down to nothing and is not the same as
/// row 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelState {
    /// The sections, as the command supplied them.
    pub sections: Vec<PanelSection>,
    /// What has been typed into the panel's own filter.
    pub filter: String,
    /// The flattened rows under that filter.
    pub rows: Vec<Row>,
    /// The selected row, or -1.
    pub selected: isize,
}

impl PanelState {
    /// Open a panel over `sections`, selecting its first action.
    #[must_use]
    pub fn new(sections: Vec<PanelSection>) -> Self {
        let rows = action_panel::flatten(&sections, "");
        let selected = action_panel::selection_after_filter(&rows);
        Self {
            sections,
            filter: String::new(),
            rows,
            selected,
        }
    }

    /// Re-filter, and put the selection back on the first row of what is left.
    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        self.rows = action_panel::flatten(&self.sections, &self.filter);
        self.selected = action_panel::selection_after_filter(&self.rows);
    }

    /// The action the selection is on, if any.
    #[must_use]
    pub fn selected_action(&self) -> Option<&Action> {
        let row = self.rows.get(usize::try_from(self.selected).ok()?)?;
        self.sections.get(row.section)?.actions.get(row.action?)
    }
}

/// The actions offered for an application.
///
/// Launching is first because it is what the return key does, and the panel's
/// first row is the one the return key runs -- so the panel opening does not
/// change what enter means.
#[must_use]
pub fn actions_for_app(item: &AppItem) -> Vec<PanelSection> {
    let mut copy = vec![Action::new("Copy name")];
    if item.path().is_some() {
        copy.push(Action::new("Copy path"));
    }
    vec![
        PanelSection {
            name: String::new(),
            actions: vec![Action::new("Open").with_shortcut("enter")],
        },
        PanelSection {
            name: "Copy".to_owned(),
            actions: copy,
        },
    ]
}

/// The launcher application state.
pub struct LauncherApp {
    /// The application index.
    app_index: AppIndex,
    /// Current query text.
    query: String,
    /// Ranked results, as indices into `app_index.items()`.
    ///
    /// Indices rather than cloned items: the ranking already works in index
    /// space, an `AppItem` carries its whole parsed desktop entry, and a
    /// launcher re-ranks on every keystroke.
    results: Vec<usize>,
    /// Which row is selected, as a position in `results`.
    selected: usize,
    /// The last launch failure, shown until the query changes.
    error: Option<String>,
    /// How to launch. See [`AppFlags::launcher`].
    launcher: Arc<dyn AppLauncher>,
    /// The engine driving this window. See [`AppFlags::link`].
    link: Option<EngineLink>,
    /// The open window, if one is.
    ///
    /// `None` is the hidden state: on Wayland a hidden window is a closed one.
    window: Option<window::Id>,
    /// Settings to open a window with, kept for every summon after the first.
    window_config: window::Settings,
    /// Which palette to draw with. See [`LauncherApp::theme`].
    appearance: Appearance,
    /// Where later appearance changes arrive. See [`AppFlags::appearance_link`].
    appearance_link: Option<crate::appearance::AppearanceLink>,
    /// Whether the selection wraps at the ends. See
    /// [`compass_core::list_navigation`].
    wrap_navigation: bool,
    /// Which navigation chords are in force. See [`compass_core::keybinding`].
    ///
    /// Held rather than read per keystroke because it comes from the user's
    /// configuration, which is read once.
    keybinding: compass_core::keybinding::Scheme,
    /// The action panel, when it is open.
    ///
    /// `None` is closed. Holding the whole state rather than a bare flag is
    /// what lets the panel keep its own filter and selection while the list
    /// underneath keeps its own -- they are two lists on screen at once, and
    /// sharing either would make one of them jump when the other moved.
    panel: Option<PanelState>,
    /// Whether the engine is waiting for an outcome right now.
    ///
    /// # Every report must answer a command, or the stream goes out of step
    ///
    /// The link is strictly one command, one outcome: the bridge sends a
    /// command and then blocks reading exactly one reply. So an outcome sent
    /// when nothing was asked does not go nowhere -- it sits in the channel and
    /// becomes the answer to the *next* command, and every answer after that is
    /// one behind, permanently.
    ///
    /// Two things used to do exactly that. Opening the window at boot reported
    /// `Shown`, and a user pressing Escape reported `Hidden`; neither answers
    /// anything. The engine would then report "shown" for a toggle that hid the
    /// window -- the precise lie this whole design exists to prevent, arriving
    /// through the mechanism built to prevent it.
    awaiting: bool,
}

/// What a dismissal does. See [`LauncherApp::on_dismiss`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dismissal {
    /// Close the window and wait to be summoned again.
    Hide,
    /// End the process, because nothing could summon it back.
    Exit,
}

/// Where the selection lands after moving one row in `direction`.
///
/// `wrap` is `launcher.wrap_navigation`, which is **false** by default -- the
/// C++ `Config::wrapNavigation` is, and the selection clamps at the first and
/// last row. An earlier version of this function wrapped unconditionally, with
/// a comment saying that is "what every launcher does"; the launcher being
/// ported does not.
///
/// Returns 0 for an empty list so callers never index into nothing.
#[must_use]
pub fn next_selection(len: usize, current: usize, direction: Direction, wrap: bool) -> usize {
    use compass_core::list_navigation::{Step, next};

    if len == 0 {
        return 0;
    }
    // Clamp the incoming index first: the list may have shrunk since it was
    // set, and `next` would otherwise step from somewhere that is not there.
    let current = current.min(len - 1);
    match direction {
        Direction::Down => next(current, Step::Forward, len, wrap),
        Direction::Up => next(current, Step::Backward, len, wrap),
    }
}

/// The direction an iced key press moves the selection, under `scheme`.
///
/// Returns `None` for `Left` and `Right` as well as for a key that is not a
/// chord: the results list is one column, so `Ctrl+H` and `Ctrl+L` have
/// nowhere to go here. They are recognised by `compass-core` and dropped here
/// deliberately, rather than being quietly bound to something they do not
/// mean.
fn chord_direction(
    scheme: compass_core::keybinding::Scheme,
    key: iced::keyboard::Key<&str>,
    modifiers: iced::keyboard::Modifiers,
) -> Option<Direction> {
    use compass_core::keybinding::{Chord, Modifiers as CoreModifiers, navigation};

    let iced::keyboard::Key::Character(text) = key else {
        return None;
    };
    let mut chars = text.chars();
    let (character, None) = (chars.next()?, chars.next()) else {
        return None;
    };

    let chord = Chord::new(
        character,
        CoreModifiers {
            // iced reports the physical Control key as `control` on every
            // platform, which is what these chords want.
            ctrl: modifiers.control(),
            alt: modifiers.alt(),
            shift: modifiers.shift(),
            logo: modifiers.logo(),
        },
    );

    match navigation(scheme, chord)? {
        compass_core::keybinding::Direction::Up => Some(Direction::Up),
        compass_core::keybinding::Direction::Down => Some(Direction::Down),
        compass_core::keybinding::Direction::Left | compass_core::keybinding::Direction::Right => {
            None
        }
    }
}

impl LauncherApp {
    /// Create a new launcher application, indexing the environment.
    pub fn new(flags: AppFlags) -> (Self, Task<Message>) {
        let mut app = Self::with_index(AppIndex::from_environment());
        app.launcher = flags.launcher;
        app.window_config = flags.window_config;
        app.keybinding = flags.keybinding;
        app.wrap_navigation = flags.wrap_navigation;
        app.link = flags.link;
        app.appearance = flags.appearance;
        app.appearance_link = flags.appearance_link;
        (app, Task::none())
    }

    /// Builds the state and opens the first window, for [`crate::run_resident`].
    ///
    /// Distinct from [`LauncherApp::new`] because `iced::daemon` starts with no
    /// windows at all: without this, `vicinae ui` with no engine attached would
    /// be an invisible process with no way to summon it.
    pub fn boot(flags: AppFlags) -> (Self, Task<Message>) {
        let (app, task) = Self::new(flags);
        let (_id, opened) = window::open(app.window_config.clone());
        (app, Task::batch([task, opened.map(Message::Opened)]))
    }

    /// Create one over a supplied index.
    ///
    /// Exists so tests can drive the state machine over a known corpus instead
    /// of whatever applications the machine running them happens to have.
    #[must_use]
    pub fn with_index(app_index: AppIndex) -> Self {
        Self {
            app_index,
            query: String::new(),
            results: Vec::new(),
            selected: 0,
            panel: None,
            error: None,
            launcher: Arc::new(NullLauncher),
            link: None,
            window: None,
            window_config: AppFlags::default().window_config,
            appearance: Appearance::Dark,
            appearance_link: None,
            keybinding: compass_core::keybinding::Scheme::default(),
            wrap_navigation: compass_core::config::DEFAULT_WRAP_NAVIGATION,
            awaiting: false,
        }
    }

    /// Whether a window is currently on screen.
    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.window.is_some()
    }

    /// Attaches an engine link. For tests that drive the resident path.
    #[must_use]
    pub fn with_link(mut self, link: EngineLink) -> Self {
        self.link = Some(link);
        self
    }

    /// Answers the engine, if it is waiting for one.
    ///
    /// Does nothing when no command is outstanding. See [`Self::awaiting`] for
    /// why that matters more than it looks.
    fn answer(&mut self, outcome: UiOutcome) {
        if !self.awaiting {
            return;
        }
        self.awaiting = false;
        if let Some(link) = &self.link {
            link.report(outcome);
        }
    }

    /// Whether the engine is waiting for an outcome. For tests.
    #[must_use]
    pub fn is_awaiting(&self) -> bool {
        self.awaiting
    }

    /// What dismissing does, given whether anything could bring the window back.
    ///
    /// Split out from [`LauncherApp::conceal`] because the two outcomes it
    /// chooses between are both opaque `Task`s: a test can see that the window
    /// closed, but not that the process was told to exit. This is the decision
    /// itself, and it is what the tests assert on.
    #[must_use]
    pub fn on_dismiss(&self) -> Dismissal {
        if self.link.is_some() {
            Dismissal::Hide
        } else {
            Dismissal::Exit
        }
    }

    /// Hides the window, or ends the process when nothing could summon it back.
    ///
    /// This is what dismissing and a successful launch both do. A launcher that
    /// stayed on screen after launching is a bug report waiting to happen; a
    /// launcher that vanished with no way back is a worse one.
    fn conceal(&mut self) -> Task<Message> {
        if self.on_dismiss() == Dismissal::Exit {
            return iced::exit();
        }
        if self.link.is_none() {
            // Unreachable: `on_dismiss` returns `Hide` only when there is a
            // link. Written as a return rather than an unwrap so a future
            // change to `on_dismiss` degrades into exiting rather than
            // panicking in the middle of a keystroke.
            return iced::exit();
        }
        match self.window {
            // The answer waits for `Message::Closed`, which arrives when the
            // window is actually gone.
            //
            // REPORTING HERE WOULD BE OPTIMISTIC, AND IT WAS. `window::close`
            // returns a Task; answering before it runs tells the engine
            // "hidden" while the window is still on screen. A VM run caught it:
            // `vicinae toggle` reported success and the screenshot taken
            // straight afterwards still had the launcher in it.
            //
            // `self.window` is deliberately NOT cleared yet. Until the close
            // lands the window really is still visible, and `is_visible` should
            // say so -- the engine cannot send another command in the meantime
            // because it is blocked reading this one's reply.
            Some(id) => window::close(id),
            // Nothing to close, so nothing to wait for. Still answers, because
            // the engine is blocked until it hears something.
            None => {
                self.answer(UiOutcome::Hidden);
                Task::none()
            }
        }
    }

    /// Replace the launcher. For tests that assert what the UI asked for.
    pub fn with_launcher(mut self, launcher: Arc<dyn AppLauncher>) -> Self {
        self.launcher = launcher;
        self
    }

    /// The item the selection currently points at, if any.
    #[must_use]
    pub fn selected_item(&self) -> Option<&AppItem> {
        let index = *self.results.get(self.selected)?;
        self.app_index.items().get(index)
    }

    /// The application title.
    pub fn title(&self) -> String {
        "Vicinae".to_owned()
    }

    /// The application theme.
    ///
    /// Adwaita's palette rather than one of Iced's built-ins, so the launcher
    /// looks like the desktop it sits on. [`crate::design`] holds the colours
    /// and the browser surrogate under `tools/design/` is served the same
    /// ones, which is what keeps the two renderings honest about each other.
    ///
    /// The appearance follows the desktop when something is feeding
    /// [`AppFlags::appearance_link`], and otherwise stays on
    /// [`AppFlags::appearance`] for the window's whole life.
    pub fn theme(&self) -> Theme {
        design::theme(self.appearance)
    }

    /// Every keyboard event, consumed by a widget or not.
    ///
    /// THIS DELIBERATELY DOES NOT USE `iced::keyboard::listen`, and the reason
    /// is a regression that focusing the search field would otherwise have
    /// introduced.
    ///
    /// `listen` yields only events with `Status::Ignored`. That was fine while
    /// nothing focused the search field, because an unfocused `text_input`
    /// consumes nothing -- which is also why the launcher ignored the keyboard
    /// entirely (#91). Focusing it fixes the typing and changes this: a focused
    /// `text_input` handles Escape by unfocusing itself and CAPTURING the
    /// event, so under `listen` the first Escape would silently stop the user
    /// typing instead of dismissing the launcher, and only a second one would
    /// reach here. Arrows and printable keys are unaffected either way -- the
    /// field captures neither.
    ///
    /// `listen_with` sees events whatever their status, so Escape means dismiss
    /// on the first press. Nothing is handled twice as a result: `update` acts
    /// on the arrows and Escape and drops every other key, while the text the
    /// field consumes arrives separately through `on_input`.
    ///
    /// Note that this is not covered by a test, and cannot easily be: what
    /// broke would be event DELIVERY, inside the Iced runtime, not the handler
    /// in `update`. The VM tier types but never presses Escape, so it would not
    /// catch it either.
    pub fn subscription(&self) -> iced::Subscription<Message> {
        let mut streams = vec![
            iced::event::listen_with(keyboard_events),
            window::close_events().map(Message::Closed),
        ];
        if let Some(link) = &self.link {
            streams.push(link.subscription().map(Message::Command));
        }
        if let Some(link) = &self.appearance_link {
            streams.push(link.subscription().map(Message::AppearanceChanged));
        }
        iced::Subscription::batch(streams)
    }

    /// Acts on a command from the engine and reports what happened.
    ///
    /// `Show` on an already-visible window reports `Shown` without opening a
    /// second one: the outcome names the state the window ended in, so
    /// "already there" and "just opened" are the same answer.
    fn obey(&mut self, command: UiCommand) -> Task<Message> {
        // From here until the outcome is sent, the engine is blocked reading
        // one reply. Set before any branch so every path answers exactly once.
        self.awaiting = true;

        let show = match command {
            UiCommand::Show => true,
            UiCommand::Hide => false,
            UiCommand::Toggle => !self.is_visible(),
        };

        if !show {
            return self.conceal();
        }

        if let Some(id) = self.window {
            self.answer(UiOutcome::Shown);
            // Both, and they are not the same thing: `gain_focus` raises the
            // WINDOW, `focus_search` focuses the FIELD inside it. A window
            // summoned with only the first is on top and still deaf.
            return Task::batch([window::gain_focus(id), focus_search()]);
        }

        let (_id, opened) = window::open(self.window_config.clone());
        opened.map(Message::Opened)
    }

    /// Update the application state.
    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Initialize => Task::none(),
            Message::AppearanceChanged(appearance) => {
                self.appearance = appearance;
                Task::none()
            }
            Message::QueryChanged(query) => {
                self.query = query;
                self.error = None;
                self.search();
                Task::none()
            }
            Message::ResultSelected(index) => {
                if index < self.results.len() {
                    self.selected = index;
                }
                Task::none()
            }
            Message::MoveSelection(direction) => {
                self.selected = next_selection(
                    self.results.len(),
                    self.selected,
                    direction,
                    self.wrap_navigation,
                );
                Task::none()
            }
            Message::LaunchSelected => {
                let Some(item) = self.selected_item() else {
                    return Task::none();
                };
                // Cloned into the future because the launch outlives this
                // borrow of `self`. An AppItem is a parsed desktop entry, so
                // this is not free — but it happens once per launch, not once
                // per keystroke.
                let entry = item.entry().clone();
                let launcher = Arc::clone(&self.launcher);
                Task::perform(
                    async move {
                        launcher
                            .launch(&entry, &[])
                            .await
                            .map(|_method| ())
                            .map_err(|err| err.to_string())
                    },
                    Message::Launched,
                )
            }
            // A launcher that stays open after launching is a bug report
            // waiting to happen. Hidden, not gone -- see `conceal`.
            Message::Launched(Ok(())) => self.conceal(),
            Message::Launched(Err(err)) => {
                self.error = Some(err);
                Task::none()
            }
            Message::Dismiss => self.conceal(),
            Message::ShortcutActivated(_) => Task::none(),
            Message::FocusChanged(_) => Task::none(),
            Message::WindowClosed => self.conceal(),
            Message::Quit => iced::exit(),
            Message::Opened(id) => {
                self.window = Some(id);
                // Answers only a `Show` that asked for it. The window opened at
                // boot answers nothing -- see `awaiting`.
                self.answer(UiOutcome::Shown);
                // Covers boot and every summon: `conceal` closes the window, so
                // a summon opens a new one whose field starts unfocused.
                focus_search()
            }
            Message::Closed(id) => {
                // Only clear the state if *this* window is the one that went;
                // a stale close for a window already replaced would otherwise
                // leave the launcher believing it is hidden while it is not.
                if self.window == Some(id) {
                    self.window = None;
                    // The honest moment to say "hidden": the window is gone.
                    // Answers only a command that asked -- a window the user
                    // closed answers nothing. See `awaiting`.
                    self.answer(UiOutcome::Hidden);
                }
                Task::none()
            }
            Message::TogglePanel => {
                if self.panel.is_some() {
                    self.panel = None;
                } else if let Some(item) = self.selected_item() {
                    // Only over a selected row. A panel of actions for nothing
                    // would be a panel whose every action fails.
                    self.panel = Some(PanelState::new(actions_for_app(item)));
                }
                Task::none()
            }
            Message::PanelFilterChanged(filter) => {
                if let Some(panel) = self.panel.as_mut() {
                    panel.set_filter(filter);
                }
                Task::none()
            }
            Message::PanelMove(direction) => {
                if let Some(panel) = self.panel.as_mut() {
                    let step = match direction {
                        Direction::Up => Step::Up,
                        Direction::Down => Step::Down,
                    };
                    panel.selected = action_panel::next_selectable(
                        &panel.rows,
                        panel.selected,
                        step,
                        self.wrap_navigation,
                    );
                }
                Task::none()
            }
            Message::PanelActivate => {
                let Some(panel) = self.panel.as_ref() else {
                    return Task::none();
                };
                let Some(action) = panel.selected_action() else {
                    return Task::none();
                };
                // Only `Open` does anything yet; the copies need a clipboard
                // this crate does not have. Closing the panel either way is
                // deliberate -- an action that ran and one that is not wired up
                // both leave the panel with nothing more to say, and leaving it
                // open would look like the key had not registered.
                let launches = action.title == "Open";
                self.panel = None;
                if launches {
                    return self.update(Message::LaunchSelected);
                }
                Task::none()
            }
            Message::Command(command) => self.obey(command),
            Message::PollShortcuts => Task::none(),
            Message::EventOccurred(_) => Task::none(),
            Message::Keyboard(iced::keyboard::Event::KeyPressed {
                ref key, modifiers, ..
            }) => {
                use iced::keyboard::{Key, key::Named};

                // Ctrl+B opens and closes the panel, over the list either way.
                //
                // Ctrl+B and not Ctrl+K, which is what the C++ binds on macOS
                // only: on Linux Ctrl+K is the vim chord for "move up", and
                // taking it here would have broken navigation for every user
                // of the default scheme. `keybind-manager.cpp` has the same
                // `#ifdef`, for the same reason.
                if modifiers.control() && key.as_ref() == Key::Character("b") {
                    return self.update(Message::TogglePanel);
                }

                // While the panel is open it takes the keys the list would
                // otherwise take. Escape closes the panel rather than the
                // launcher, because a panel opened by mistake should cost one
                // key and not the whole window.
                if self.panel.is_some() {
                    match key.as_ref() {
                        Key::Named(Named::ArrowDown) => {
                            return self.update(Message::PanelMove(Direction::Down));
                        }
                        Key::Named(Named::ArrowUp) => {
                            return self.update(Message::PanelMove(Direction::Up));
                        }
                        Key::Named(Named::Enter) => return self.update(Message::PanelActivate),
                        Key::Named(Named::Escape) => {
                            self.panel = None;
                            return Task::none();
                        }
                        _ => {}
                    }
                    return match chord_direction(self.keybinding, key.as_ref(), modifiers) {
                        Some(direction) => self.update(Message::PanelMove(direction)),
                        None => Task::none(),
                    };
                }

                match key.as_ref() {
                    Key::Named(Named::ArrowDown) => {
                        return self.update(Message::MoveSelection(Direction::Down));
                    }
                    Key::Named(Named::ArrowUp) => {
                        return self.update(Message::MoveSelection(Direction::Up));
                    }
                    Key::Named(Named::Escape) => return self.update(Message::Dismiss),
                    _ => {}
                }

                // A chord, if this one is. The scheme decides; `compass-core`
                // owns which chords each scheme has, so this front end does
                // not have a second opinion about it.
                match chord_direction(self.keybinding, key.as_ref(), modifiers) {
                    Some(direction) => self.update(Message::MoveSelection(direction)),
                    None => Task::none(),
                }
            }
            Message::Keyboard(_) => Task::none(),
        }
    }

    /// View the application.
    ///
    /// A card: a search field over a list of rows, each row an icon, a title
    /// and a subtitle, with the selection drawn as a filled rounded rectangle
    /// rather than a caret.
    ///
    /// **The caret is gone and that is deliberate.** It was there because a
    /// comment said "under llvmpipe at 1280x800 a background tint is not
    /// identifiable in a captured frame". `framediff.py` compares raw RGB
    /// bytes for exact inequality, with no threshold, so a tinted row of
    /// roughly 600x30 is about 18,000 changed pixels -- some seven times the
    /// 2,697 the action-panel assertion already detects reliably. A highlight
    /// is *easier* for the tier to see than a caret, not harder.
    pub fn view(&self) -> Element<'_, Message> {
        let palette = design::palette(self.appearance);

        let input = text_input("Search…", &self.query)
            .id(SEARCH_INPUT)
            .on_input(Message::QueryChanged)
            .padding(Padding::new(0.0).left(14).right(14))
            .size(f32::from(GEOMETRY.query_size))
            .on_submit(Message::LaunchSelected);

        let field = container(input)
            .height(Length::Fixed(f32::from(GEOMETRY.field_height)))
            .width(Length::Fill)
            .align_y(Alignment::Center)
            .style(move |_: &Theme| container::Style {
                background: Some(palette.field.to_iced().into()),
                border: Border {
                    color: palette.border.to_iced(),
                    width: 1.0,
                    radius: f32::from(GEOMETRY.field_radius).into(),
                },
                ..container::Style::default()
            });

        let body: Element<Message> = if let Some(err) = &self.error {
            self.notice(&format!("could not launch: {err}"))
        } else if self.query.is_empty() {
            self.notice("Type to search")
        } else if self.results.is_empty() {
            self.notice("No results")
        } else {
            let mut list = column![].spacing(f32::from(GEOMETRY.row_spacing));
            for (position, index) in self.results.iter().enumerate() {
                let Some(item) = self.app_index.items().get(*index) else {
                    continue;
                };
                list = list.push(self.result_row(item, position == self.selected));
            }
            container(list).padding(Padding::new(6.0).top(8)).into()
        };

        let card_content = column![field, body].width(Length::Fill);

        // The panel floats over the list rather than replacing it. The old
        // comment said an overlay "needs a stacking widget and a backdrop" --
        // `stack!` is that widget, and the backdrop turned out to be
        // unnecessary because the panel is opaque and bounded.
        let card_body: Element<Message> = match &self.panel {
            Some(panel) => stack![
                card_content,
                container(self.view_panel(panel))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_x(Alignment::End)
                    .align_y(Alignment::End)
                    .padding(8)
            ]
            .into(),
            None => card_content.into(),
        };

        container(
            container(card_body)
                .width(Length::Fixed(f32::from(GEOMETRY.card_width)))
                .padding(GEOMETRY.card_padding)
                .style(move |_: &Theme| container::Style {
                    background: Some(palette.surface.to_iced().into()),
                    border: Border {
                        color: palette.border.to_iced(),
                        width: 1.0,
                        radius: f32::from(GEOMETRY.card_radius).into(),
                    },
                    ..container::Style::default()
                }),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::Center)
        .into()
    }

    /// A line of explanation where the list would be.
    fn notice(&self, message: &str) -> Element<'_, Message> {
        let palette = design::palette(self.appearance);
        container(
            text(message.to_owned())
                .size(f32::from(GEOMETRY.title_size))
                .color(palette.muted.to_iced()),
        )
        .width(Length::Fill)
        .padding(28)
        .align_x(Alignment::Center)
        .into()
    }

    /// One result: icon, title, subtitle.
    ///
    /// The icon is the first letter in a tinted square. `AppItem` has an icon
    /// *name*, and resolving it through the XDG icon theme is its own piece of
    /// work; a letter at the right size keeps the row's proportions honest
    /// until then, and is what the design surrogate draws for the same reason.
    fn result_row(&self, item: &AppItem, selected: bool) -> Element<'_, Message> {
        let palette = design::palette(self.appearance);
        let title_color = if selected {
            palette.selection_text
        } else {
            palette.text
        };
        let subtitle_color = if selected {
            palette.selection_text
        } else {
            palette.muted
        };

        let initial = item
            .name()
            .chars()
            .next()
            .map_or_else(String::new, |c| c.to_uppercase().to_string());

        let icon = container(
            text(initial)
                .size(f32::from(GEOMETRY.icon_size) / 2.0)
                .color(title_color.to_iced()),
        )
        .width(Length::Fixed(f32::from(GEOMETRY.icon_size)))
        .height(Length::Fixed(f32::from(GEOMETRY.icon_size)))
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(move |_: &Theme| container::Style {
            background: Some(
                Color {
                    a: 0.18,
                    ..palette.accent.to_iced()
                }
                .into(),
            ),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 8.0.into(),
            },
            ..container::Style::default()
        });

        let mut labels = column![
            text(item.name().to_owned())
                .size(f32::from(GEOMETRY.title_size))
                .color(title_color.to_iced())
        ];
        if let Some(comment) = item.comment() {
            labels = labels.push(
                text(comment.to_owned())
                    .size(f32::from(GEOMETRY.subtitle_size))
                    .color(subtitle_color.to_iced()),
            );
        }

        container(
            row![icon, labels]
                .spacing(12)
                .align_y(Alignment::Center)
                .padding(Padding::new(0.0).left(12).right(12)),
        )
        .width(Length::Fill)
        .height(Length::Fixed(f32::from(GEOMETRY.row_height)))
        .style(move |_: &Theme| {
            if selected {
                container::Style {
                    background: Some(palette.selection.to_iced().into()),
                    border: Border {
                        color: Color::TRANSPARENT,
                        width: 0.0,
                        radius: f32::from(GEOMETRY.row_radius).into(),
                    },
                    ..container::Style::default()
                }
            } else {
                container::Style::default()
            }
        })
        .into()
    }

    /// Draw the action panel.
    ///
    /// A floating card at the bottom right, the way Raycast's sits. Headers
    /// and dividers are drawn as themselves rather than as indented text, and
    /// the selection is the same filled rectangle the result list uses.
    fn view_panel(&self, panel: &PanelState) -> Element<'_, Message> {
        let palette = design::palette(self.appearance);
        let mut col = column![].spacing(f32::from(GEOMETRY.row_spacing));

        for (index, panel_row) in panel.rows.iter().enumerate() {
            let element: Element<Message> = match panel_row.kind {
                RowKind::Divider => container(Space::new().height(Length::Fixed(1.0)))
                    .width(Length::Fill)
                    .padding(Padding::new(0.0).top(5).bottom(5).left(8).right(8))
                    .style(move |_: &Theme| container::Style {
                        background: Some(palette.border.to_iced().into()),
                        ..container::Style::default()
                    })
                    .into(),
                RowKind::Header => {
                    let name = panel
                        .sections
                        .get(panel_row.section)
                        .map_or("", |section| section.name.as_str());
                    container(
                        text(name.to_uppercase())
                            .size(f32::from(GEOMETRY.heading_size))
                            .color(palette.muted.to_iced()),
                    )
                    .padding(Padding::new(0.0).top(8).bottom(4).left(10))
                    .into()
                }
                RowKind::Item => {
                    let action = panel_row.action.and_then(|position| {
                        panel.sections.get(panel_row.section)?.actions.get(position)
                    });
                    let title = action.map_or("", |action| action.title.as_str());
                    let shortcut = action.and_then(|action| action.shortcut.clone());
                    let selected = isize::try_from(index).unwrap_or(isize::MAX) == panel.selected;
                    self.panel_item(title, shortcut.as_deref(), selected)
                }
            };
            col = col.push(element);
        }

        if panel.rows.is_empty() {
            col = col.push(self.notice("No actions"));
        }

        container(col)
            .width(Length::Fixed(300.0))
            .padding(6)
            .style(move |_: &Theme| container::Style {
                background: Some(palette.surface.to_iced().into()),
                border: Border {
                    color: palette.border.to_iced(),
                    width: 1.0,
                    radius: 12.0.into(),
                },
                ..container::Style::default()
            })
            .into()
    }

    /// One action row, with its shortcut right-aligned.
    fn panel_item(
        &self,
        title: &str,
        shortcut: Option<&str>,
        selected: bool,
    ) -> Element<'_, Message> {
        let palette = design::palette(self.appearance);
        let colour = if selected {
            palette.selection_text
        } else {
            palette.text
        };

        let mut line = row![
            text(title.to_owned())
                .size(f32::from(GEOMETRY.title_size))
                .color(colour.to_iced())
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        if let Some(shortcut) = shortcut {
            line = line.push(Space::new().width(Length::Fill));
            line = line.push(
                text(shortcut.to_owned())
                    .size(f32::from(GEOMETRY.subtitle_size))
                    .color(
                        if selected {
                            palette.selection_text
                        } else {
                            palette.muted
                        }
                        .to_iced(),
                    ),
            );
        }

        container(line.padding(Padding::new(0.0).left(10).right(10)))
            .width(Length::Fill)
            .height(Length::Fixed(34.0))
            .style(move |_: &Theme| {
                if selected {
                    container::Style {
                        background: Some(palette.selection.to_iced().into()),
                        border: Border {
                            color: Color::TRANSPARENT,
                            width: 0.0,
                            radius: 8.0.into(),
                        },
                        ..container::Style::default()
                    }
                } else {
                    container::Style::default()
                }
            })
            .into()
    }

    /// Re-rank against the current query.
    fn search(&mut self) {
        if self.query.trim().is_empty() {
            self.results.clear();
            self.selected = 0;
            return;
        }

        self.results = rank_indices(&self.query, self.app_index.items())
            .into_iter()
            .map(|scored| scored.item)
            .collect();
        // Back to the top on every new query: the old selection pointed into a
        // different list, and keeping its position would silently select an
        // unrelated application.
        self.selected = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// An index over three entries written to a tempdir, so the tests do not
    /// depend on what the machine running them has installed.
    fn index(dir: &std::path::Path) -> AppIndex {
        for (file, name) in [
            ("firefox.desktop", "Firefox"),
            ("files.desktop", "Files"),
            ("terminal.desktop", "Terminal"),
        ] {
            fs::write(
                dir.join(file),
                format!("[Desktop Entry]\nType=Application\nName={name}\nExec=/bin/true\n"),
            )
            .expect("write entry");
        }
        AppIndex::builder().dir(dir).build()
    }

    fn app(dir: &std::path::Path) -> LauncherApp {
        LauncherApp::with_index(index(dir))
    }

    #[test]
    fn the_selection_clamps_by_default_because_the_cpp_does() {
        // This used to assert the opposite. `Config::wrapNavigation` is
        // `false` in the engine being ported, so Up at the top stays at the
        // top -- and `compass_core::list_navigation` holds the rule.
        assert_eq!(next_selection(3, 0, Direction::Up, false), 0);
        assert_eq!(next_selection(3, 2, Direction::Down, false), 2);
        assert_eq!(next_selection(3, 0, Direction::Down, false), 1);
        assert_eq!(next_selection(3, 2, Direction::Up, false), 1);
    }

    /// Themes compare by value, and a failed comparison prints two whole
    /// palettes. The name is what distinguishes ours, so assert on that.
    fn theme_name(app: &LauncherApp) -> String {
        app.theme().to_string()
    }

    #[test]
    fn an_appearance_change_reaches_the_theme() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        assert_eq!(
            theme_name(&app),
            design::theme(Appearance::Dark).to_string()
        );

        let _ = app.update(Message::AppearanceChanged(Appearance::Light));
        // Asserted through `theme()` rather than the field, because the field
        // being right while the theme is built from something else is exactly
        // the bug worth catching.
        assert_eq!(
            theme_name(&app),
            design::theme(Appearance::Light).to_string()
        );
    }

    #[test]
    fn a_window_nobody_is_feeding_keeps_the_appearance_it_started_with() {
        let (mut app, _task) = LauncherApp::new(AppFlags {
            appearance: Appearance::Light,
            appearance_link: None,
            ..AppFlags::default()
        });

        // No portal, no feeder: whatever it opened as is what it stays as. A
        // launcher that fell back to dark here would change colour on every
        // desktop without a Settings portal.
        let opened_as = theme_name(&app);
        assert_eq!(opened_as, design::theme(Appearance::Light).to_string());
        let _ = app.update(Message::QueryChanged("fir".to_owned()));
        let _ = app.update(Message::TogglePanel);
        assert_eq!(theme_name(&app), opened_as);
    }

    #[test]
    fn the_starting_appearance_is_what_the_flags_say() {
        for appearance in Appearance::ALL {
            let (app, _task) = LauncherApp::new(AppFlags {
                appearance,
                ..AppFlags::default()
            });
            assert_eq!(theme_name(&app), design::theme(appearance).to_string());
        }
    }

    #[test]
    fn the_selection_wraps_when_the_setting_asks_for_it() {
        assert_eq!(next_selection(3, 0, Direction::Up, true), 2);
        assert_eq!(next_selection(3, 2, Direction::Down, true), 0);
    }

    #[test]
    fn selection_on_an_empty_list_is_not_an_index_into_nothing() {
        assert_eq!(next_selection(0, 0, Direction::Up, false), 0);
        assert_eq!(next_selection(0, 0, Direction::Down, true), 0);
    }

    #[test]
    fn a_selection_left_past_the_end_is_clamped_rather_than_underflowing() {
        // Reachable if a list shrinks without the selection being reset. The
        // arithmetic here is unsigned, so getting this wrong is a panic.
        assert_eq!(next_selection(2, 9, Direction::Up, false), 0);
        assert_eq!(next_selection(1, 5, Direction::Down, false), 0);
    }

    #[test]
    fn typing_ranks_and_selects_the_first_row() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("fire".to_owned()));
        assert_eq!(app.selected, 0);
        assert_eq!(
            app.selected_item().map(compass_core::AppItem::name),
            Some("Firefox")
        );
    }

    #[test]
    fn a_new_query_resets_the_selection() {
        // Otherwise the old position points into a different list and the user
        // launches something they never looked at.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        // "fi" matches Files and Firefox. A single "e" matches nothing at all:
        // the coherence rule (ADR-0006) drops a one-character query that is not
        // a word prefix, which is correct and cost this test a first draft.
        let _ = app.update(Message::QueryChanged("fi".to_owned()));
        let _ = app.update(Message::MoveSelection(Direction::Down));
        assert_ne!(app.selected, 0, "precondition: the selection moved");
        let _ = app.update(Message::QueryChanged("fire".to_owned()));
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn an_empty_query_shows_nothing_and_selects_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("fire".to_owned()));
        let _ = app.update(Message::QueryChanged("   ".to_owned()));
        assert!(app.results.is_empty());
        assert!(app.selected_item().is_none());
    }

    #[test]
    fn launching_with_no_results_does_nothing_rather_than_panicking() {
        // Enter on an empty list is the most ordinary way to reach this.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("zzzznotathing".to_owned()));
        assert!(app.results.is_empty());
        let _ = app.update(Message::LaunchSelected);
    }

    #[test]
    fn a_failed_launch_is_shown_and_cleared_by_the_next_keystroke() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::Launched(Err("no Exec key".to_owned())));
        assert_eq!(app.error.as_deref(), Some("no Exec key"));
        let _ = app.update(Message::QueryChanged("f".to_owned()));
        assert!(app.error.is_none(), "a new query should clear the error");
    }

    /// Builds a `KeyPressed` for a named key, as the subscription delivers it.
    fn pressed(key: iced::keyboard::key::Named) -> Message {
        Message::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(key),
            modified_key: iced::keyboard::Key::Named(key),
            physical_key: iced::keyboard::key::Physical::Unidentified(
                iced::keyboard::key::NativeCode::Unidentified,
            ),
            location: iced::keyboard::Location::Standard,
            modifiers: iced::keyboard::Modifiers::default(),
            text: None,
            repeat: false,
        })
    }

    /// A `KeyPressed` for a character key with modifiers.
    fn chord(character: &str, modifiers: iced::keyboard::Modifiers) -> Message {
        Message::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Character(character.into()),
            modified_key: iced::keyboard::Key::Character(character.into()),
            physical_key: iced::keyboard::key::Physical::Unidentified(
                iced::keyboard::key::NativeCode::Unidentified,
            ),
            location: iced::keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        })
    }

    /// An app over the fixture corpus with several matching rows.
    fn app_with_rows(dir: &std::path::Path) -> LauncherApp {
        let mut app = app(dir);
        let _ = app.update(Message::QueryChanged("fi".to_owned()));
        assert!(app.results.len() > 1, "need several rows to move between");
        app
    }

    #[test]
    fn the_action_panel_is_not_bound_to_the_vim_chord() {
        // Ctrl+K is "move up" in the default Linux scheme. The C++ binds the
        // panel to Ctrl+K on macOS only and Ctrl+B everywhere else, and this
        // is why -- taking Ctrl+K here would break navigation for every user
        // who has configured nothing.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        app.keybinding = compass_core::keybinding::Scheme::Vim;

        let _ = app.update(chord("j", iced::keyboard::Modifiers::CTRL));
        assert_eq!(app.selected, 1, "precondition: Ctrl+J moved down");

        let _ = app.update(chord("k", iced::keyboard::Modifiers::CTRL));
        assert!(app.panel.is_none(), "Ctrl+K must not open the panel");
        assert_eq!(app.selected, 0, "Ctrl+K still moves up");
    }

    #[test]
    fn ctrl_b_opens_and_closes_the_panel() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());

        let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
        assert!(app.panel.is_some(), "Ctrl+B opened it");

        let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
        assert!(app.panel.is_none(), "and Ctrl+B closed it again");
    }

    #[test]
    fn the_panel_does_not_open_over_nothing() {
        // A panel of actions for no selected row is a panel whose every action
        // fails.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("zzzzzzzz".to_owned()));
        assert!(
            app.selected_item().is_none(),
            "precondition: nothing selected"
        );

        let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
        assert!(app.panel.is_none());
    }

    #[test]
    fn the_panel_takes_the_arrow_keys_while_it_is_open() {
        // Otherwise the list underneath moves out from under a panel whose
        // actions are for the row that was selected when it opened.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
        let before = app.selected;

        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
        assert_eq!(app.selected, before, "the list did not move");
        // Row 1 is the divider and row 2 the heading, so the next selectable
        // row is 3. Expecting 1 would have been expecting the selection to
        // land on a divider.
        assert_eq!(
            app.panel.as_ref().map(|panel| panel.selected),
            Some(3),
            "the panel did"
        );
    }

    #[test]
    fn typing_the_panels_letter_without_ctrl_does_not_open_it() {
        // The search field takes ordinary characters, and a launcher whose
        // search box opens a panel when someone types `b` is unusable.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());

        let _ = app.update(chord("b", iced::keyboard::Modifiers::default()));
        assert!(app.panel.is_none());
    }

    #[test]
    fn escape_closes_the_panel_rather_than_the_launcher() {
        // A panel opened by mistake should cost one key, not the whole window.
        // The window has to be open for this to mean anything -- with no
        // window, "the window did not close" is true however Escape is routed.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let _ = app.update(Message::Opened(window::Id::unique()));
        assert!(app.window.is_some(), "precondition: a window is open");

        let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
        let _ = app.update(pressed(iced::keyboard::key::Named::Escape));
        assert!(app.panel.is_none(), "the panel closed");
        assert!(app.window.is_some(), "and the window did not");

        // HALF OF THIS IS NOT CONTROL-BACKED, and it is worth saying which.
        //
        // "The panel closed" fires under a mutation. "The window did not"
        // does not: `conceal` closes the window through a Task and clears
        // `self.window` only when `Message::Closed` comes back, and with no
        // engine link `on_dismiss` returns `Exit`, so a mutation that made
        // Escape dismiss as well as close the panel changes nothing this test
        // can see. Proving it would need an app built around a live
        // `EngineLink`, which is a harness this crate does not have yet.
        //
        // Recorded rather than left looking covered.
    }

    #[test]
    fn the_panel_stops_intercepting_once_it_is_closed() {
        // The follow-on from the test above: with the panel gone, Escape is
        // the launcher's again. What it *does* then is `conceal`, which closes
        // the window through a Task and so is not observable from here — so
        // this asserts the routing rather than the closing, and the test above
        // asserts that the routing was different while the panel was open.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let _ = app.update(Message::Opened(window::Id::unique()));
        let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
        let _ = app.update(pressed(iced::keyboard::key::Named::Escape));
        assert!(app.panel.is_none(), "the panel closed");

        // Arrows reach the list again, which the panel was taking.
        let before = app.selected;
        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
        assert_ne!(app.selected, before, "the list moved again");
    }

    #[test]
    fn the_vim_chords_move_the_panel_while_it_is_open() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        app.keybinding = compass_core::keybinding::Scheme::Vim;
        let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
        let before = app.selected;

        let _ = app.update(chord("j", iced::keyboard::Modifiers::CTRL));
        assert_eq!(app.selected, before, "the list did not move");
        assert_eq!(app.panel.as_ref().map(|panel| panel.selected), Some(3));
    }

    #[test]
    fn running_an_action_closes_the_panel() {
        // An action that ran and one that is not wired up both leave the panel
        // with nothing more to say, and leaving it open would look like the key
        // had not registered.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));

        let _ = app.update(pressed(iced::keyboard::key::Named::Enter));
        assert!(app.panel.is_none());
    }

    #[test]
    fn the_vim_chords_move_the_selection_because_they_are_the_linux_default() {
        // `KeyBindingService::getMode` falls back to vim off macOS, so a user
        // who has configured nothing still expects Ctrl+J and Ctrl+K to work.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        app.keybinding = compass_core::keybinding::Scheme::Vim;
        assert_eq!(app.selected, 0);

        let _ = app.update(chord("j", iced::keyboard::Modifiers::CTRL));
        assert_eq!(app.selected, 1, "Ctrl+J moved down");

        let _ = app.update(chord("k", iced::keyboard::Modifiers::CTRL));
        assert_eq!(app.selected, 0, "and Ctrl+K moved back");
    }

    #[test]
    fn a_bare_letter_is_not_a_chord() {
        // The negative that matters most: a handler that ignored modifiers
        // would make the search field unusable, because every `j` typed would
        // also move the selection.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        app.keybinding = compass_core::keybinding::Scheme::Vim;

        let _ = app.update(chord("j", iced::keyboard::Modifiers::empty()));
        assert_eq!(app.selected, 0, "a bare `j` moved the selection");
    }

    #[test]
    fn a_chord_from_another_scheme_does_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        app.keybinding = compass_core::keybinding::Scheme::Emacs;

        let _ = app.update(chord("j", iced::keyboard::Modifiers::CTRL));
        assert_eq!(app.selected, 0, "Ctrl+J is vim's, not emacs'");

        let _ = app.update(chord("n", iced::keyboard::Modifiers::CTRL));
        assert_eq!(app.selected, 1, "Ctrl+N is emacs' down");
    }

    #[test]
    fn the_horizontal_chords_do_nothing_in_a_one_column_list() {
        // Recognised by `compass-core`, dropped here on purpose rather than
        // bound to something they do not mean.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        app.keybinding = compass_core::keybinding::Scheme::Vim;

        let _ = app.update(chord("l", iced::keyboard::Modifiers::CTRL));
        let _ = app.update(chord("h", iced::keyboard::Modifiers::CTRL));
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn the_arrows_move_the_selection_through_the_keyboard_subscription() {
        // Goes through `Message::Keyboard` rather than `Message::MoveSelection`
        // on purpose: the direct path is covered above, and what this covers is
        // the match arm that routes a key to it. That arm is what a change to
        // the subscription puts at risk.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("fi".to_owned()));
        assert!(app.results.len() > 1, "need several rows to move between");
        assert_eq!(app.selected, 0);

        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
        assert_eq!(app.selected, 1, "ArrowDown moved down");

        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowUp));
        assert_eq!(app.selected, 0, "and ArrowUp moved back");
    }

    #[test]
    fn a_printable_key_from_the_subscription_does_not_also_type() {
        // THE CONTROL FOR HANDLING A KEY TWICE.
        //
        // `subscription` reads every keyboard event, including ones the search
        // field consumed, so that Escape survives the field being focused. The
        // risk that buys is double handling: if `update` ever grew an arm that
        // appended printable keys to the query, every character would arrive
        // twice -- once here and once through the field's `on_input` -- and
        // "fi" would be typed as "ffii".
        //
        // Nothing here can observe `on_input`, which Iced calls; what it can
        // observe is that this path contributes nothing on its own.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("fi".to_owned()));
        let before = app.query.clone();

        let _ = app.update(Message::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Character("x".into()),
            modified_key: iced::keyboard::Key::Character("x".into()),
            physical_key: iced::keyboard::key::Physical::Unidentified(
                iced::keyboard::key::NativeCode::Unidentified,
            ),
            location: iced::keyboard::Location::Standard,
            modifiers: iced::keyboard::Modifiers::default(),
            text: Some("x".into()),
            repeat: false,
        }));

        assert_eq!(app.query, before, "the query is the field's to change");
    }

    #[test]
    fn moving_the_selection_lands_on_the_item_that_gets_launched() {
        // The property that matters: what the caret points at in the view is
        // what LaunchSelected resolves. A off-by-one here launches the wrong
        // application, silently and every time.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("fi".to_owned()));
        let expected: Vec<String> = app
            .results
            .iter()
            .filter_map(|i| app.app_index.items().get(*i))
            .map(|item| item.name().to_owned())
            .collect();
        assert!(expected.len() > 1, "need several rows: {expected:?}");

        for (position, name) in expected.iter().enumerate() {
            assert_eq!(app.selected, position);
            assert_eq!(
                app.selected_item().map(compass_core::AppItem::name),
                Some(name.as_str())
            );
            let _ = app.update(Message::MoveSelection(Direction::Down));
        }
        assert_eq!(
            app.selected,
            expected.len() - 1,
            "and it stayed on the last row, because wrap_navigation is off by default"
        );
    }
}
