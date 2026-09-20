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
    widget::{Space, column, container, image, mouse_area, row, stack, svg, text, text_input},
    window,
};

use std::sync::Arc;

use compass_core::{AppIndex, AppItem};
use compass_platform::{AppLauncher, NullLauncher};

use crate::action_panel::{self, Action, PanelSection, Row, RowKind, Step};
use crate::design::{self, Appearance, GEOMETRY, TINT_ALPHA};
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
const PANEL_INPUT: &str = "compass-action-filter";
const APP_OPEN: &str = "app.open";
const APP_COPY_NAME: &str = "app.copy-name";
const APP_COPY_PATH: &str = "app.copy-path";
const APP_DESKTOP_ACTION: &str = "app.desktop:";

/// The nominal pixel size asked of the icon theme.
///
/// [`design::GEOMETRY`]'s `icon_size`, which is the square the row reserves.
/// Asking the theme for that size rather than a fixed 32 keeps the two from
/// drifting: a design change that grows the slot asks for a larger icon
/// instead of scaling a small one up.
const ICON_PIXELS: u32 = GEOMETRY.icon_size as u32;

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

/// Which result `Ctrl+1`..`Ctrl+9` asks for, as a **position in the visible
/// list** counting from zero.
///
/// # The two things this is easy to get wrong
///
/// It returns a POSITION, not an item index. `LauncherApp::results` holds
/// indices into the application index, and the selection is a position within
/// `results` -- so `Ctrl+3` means the third row on screen, not item 3. Mixing
/// those up launches something plausible and wrong, which is the worst kind of
/// bug to have here because nothing looks broken.
///
/// And there is no zero. `Ctrl+1` is the first row, so the digit is one-based
/// and `Ctrl+0` is not a quick-launch chord at all.
///
/// Bounds are the caller's: this says which row was asked for, not whether it
/// exists.
#[must_use]
pub fn quick_launch_position(
    key: iced::keyboard::Key<&str>,
    modifiers: iced::keyboard::Modifiers,
) -> Option<usize> {
    use iced::keyboard::Key;

    // Control alone. A chord carrying Alt or Logo as well is somebody else's,
    // and claiming it would take a binding the desktop may already have.
    // Shift is excluded too: Ctrl+Shift+1 is a different chord, and on many
    // layouts it is not a digit at all.
    if !modifiers.control() || modifiers.alt() || modifiers.logo() || modifiers.shift() {
        return None;
    }

    let Key::Character(text) = key else {
        return None;
    };

    // `"1".."9"`, and nothing else: not "10", not a digit with anything
    // attached. `chars().next()` on a longer string would accept "1x".
    let mut chars = text.chars();
    let digit = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    let value = digit.to_digit(10)?;
    if value == 0 {
        return None;
    }
    Some(value as usize - 1)
}

type IconFinder = dyn Fn(&str) -> Option<std::path::PathBuf> + Send + Sync;

/// Resolves an `Icon=` name to a file. See [`AppFlags::icon_lookup`].
///
/// A named type rather than a bare `Arc<dyn Fn…>` so it can carry a `Debug`
/// impl: [`AppFlags`] derives `Debug`, and a closure does not.
#[derive(Clone)]
pub struct IconLookup(Arc<IconFinder>);

impl IconLookup {
    /// Wraps a lookup.
    #[must_use]
    pub fn new(find: impl Fn(&str) -> Option<std::path::PathBuf> + Send + Sync + 'static) -> Self {
        Self(Arc::new(find))
    }

    /// The file for `name`, if the lookup can find one.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<std::path::PathBuf> {
        (self.0)(name)
    }
}

impl std::fmt::Debug for IconLookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IconLookup(..)")
    }
}

impl Default for IconLookup {
    /// The XDG icon themes, at the size the row reserves.
    fn default() -> Self {
        Self::new(|name: &str| {
            compass_xdg::find_icon(
                name,
                Some(&compass_xdg::default_theme()),
                Some(ICON_PIXELS),
                1.0,
            )
        })
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
    /// Whether Ctrl+1..9 launches the Nth result, from `launcher.quick_launch`.
    pub quick_launch: bool,
    /// When the process started, for the cold-start figure (#13).
    ///
    /// `None` in a test or anywhere nobody is timing, which simply means no
    /// figure is logged. Taken by the binary on entry rather than here, so it
    /// covers as much of the startup as a process can see of itself.
    pub started_at: Option<std::time::Instant>,
    /// The resolved appearance preset (#84): geometry and structural flags.
    pub appearance_preset: crate::preset::Resolved,
    /// Whether result rows show the application's icon, from
    /// `launcher.appearance.icons` (#85).
    ///
    /// Resolved from the preset and any explicit key, so this is the answer
    /// rather than the configured value. See [`crate::preset::resolve`].
    pub icons: bool,
    /// How an `Icon=` name becomes a file on disk.
    ///
    /// Injected for the reason [`AppFlags::launcher`] is: the default walks the
    /// XDG icon themes, so a test that did not replace it would assert against
    /// whatever the machine running it happens to have installed. See
    /// ADR-0013.
    pub icon_lookup: IconLookup,
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
            quick_launch: compass_core::config::DEFAULT_QUICK_LAUNCH,
            icons: compass_core::config::DEFAULT_ICONS,
            icon_lookup: IconLookup::default(),
            appearance_preset: crate::preset::resolve(None, None, None),
            started_at: None,
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
    let mut primary = vec![Action::new("Open").with_id(APP_OPEN).with_shortcut("enter")];
    if !item.is_action() {
        for action in item.entry().actions() {
            if let Some(name) = action.name().filter(|name| !name.is_empty())
                && action.exec().is_some()
            {
                primary.push(
                    Action::new(name).with_id(format!("{APP_DESKTOP_ACTION}{}", action.id())),
                );
            }
        }
    }
    let mut copy = vec![Action::new("Copy name").with_id(APP_COPY_NAME)];
    if item.path().is_some() {
        copy.push(Action::new("Copy path").with_id(APP_COPY_PATH));
    }
    vec![
        PanelSection {
            name: String::new(),
            actions: primary,
        },
        PanelSection {
            name: "Copy".to_owned(),
            actions: copy,
        },
    ]
}

fn launch_task(
    launcher: Arc<dyn AppLauncher>,
    entry: compass_xdg::DesktopEntry,
    action_id: Option<String>,
) -> Task<Message> {
    Task::perform(
        async move {
            match action_id {
                Some(id) => launcher.launch_action(&entry, &id, &[]).await,
                None => launcher.launch(&entry, &[]).await,
            }
            .map(|_| ())
            .map_err(|error| error.to_string())
        },
        Message::Launched,
    )
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
    /// Whether Ctrl+1..9 launches the Nth result. See [`AppFlags::quick_launch`].
    quick_launch: bool,
    /// Sizes and spacing, from the resolved appearance preset (#84).
    ///
    /// Held rather than read from [`design::GEOMETRY`] at each draw: a preset
    /// varies it, and `view` must not have to know which one is in force.
    geometry: design::Geometry,
    /// Whether a rule separates the field from the results. See
    /// [`crate::preset::Preset::field_rule`].
    field_rule: bool,

    /// Whether the card background is translucent (#86).
    ///
    /// Translucency, not blur — see [`crate::preset::Preset::tint`]. The window
    /// surface is already transparent (`AppFlags::default`), so this only
    /// changes the card's own background alpha.
    tint: bool,
    /// Whether rows show their subtitle. See [`crate::preset::Preset::subtitles`].
    subtitles: bool,
    /// When the first frame was drawn, for the cold-start figure (#13).
    ///
    /// `None` until it happens, which is also what keeps the per-frame
    /// subscription from running for the life of the process.
    first_frame_at: Option<std::time::Instant>,
    /// When the process started. See [`AppFlags::started_at`].
    started_at: Option<std::time::Instant>,
    /// Whether result rows show the application's icon. See [`AppFlags::icons`].
    icons: bool,
    /// How an `Icon=` name becomes a file. See [`AppFlags::icon_lookup`].
    icon_lookup: IconLookup,
    /// Icons resolved for rows that have been on screen (#85).
    ///
    /// Warmed from `update`, never from `view`: resolving a name walks the
    /// theme directories, which is disk work that does not belong in a draw.
    icon_cache: crate::icons::IconCache,
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

/// The card's background colour, given the surface colour and whether `tint` is on.
///
/// Extracted from `view`'s style closure on purpose. A headless renderer can
/// assert what is laid out but not what colour a container was filled with, so
/// the alpha decision inside a closure would be untestable without a pixel
/// baseline — and pixel baselines get rubber-stamped, which is why §8.5 rejects
/// them. As a function it is an ordinary assertion.
///
/// Translucency is applied here rather than in the palette so the theme stays
/// one set of colours: `tint` is a property of the window, not of the scheme,
/// and a palette that changed with it would make every other surface's
/// contrast depend on a background setting.
fn card_background(surface: design::Rgb, tint: bool) -> iced::Color {
    let colour = surface.to_iced();
    if tint {
        colour.scale_alpha(TINT_ALPHA)
    } else {
        colour
    }
}

impl LauncherApp {
    /// Create a new launcher application, indexing the environment.
    pub fn new(flags: AppFlags) -> (Self, Task<Message>) {
        let mut app = Self::with_index(AppIndex::from_environment());
        app.apply(flags);
        (app, Task::none())
    }

    /// Copy the flags onto an already-built state.
    ///
    /// Split out of [`LauncherApp::new`] so the copying is reachable without
    /// `AppIndex::from_environment`, which reads the invoking user's real
    /// application directories. A test that reproduced this assignment list
    /// instead of calling it would pass while the real one stopped copying a
    /// field -- which is exactly the mutation it is meant to catch.
    fn apply(&mut self, flags: AppFlags) {
        let app = self;
        app.launcher = flags.launcher;
        app.window_config = flags.window_config;
        app.keybinding = flags.keybinding;
        app.wrap_navigation = flags.wrap_navigation;
        app.quick_launch = flags.quick_launch;
        app.icons = flags.icons;
        app.geometry = flags.appearance_preset.geometry;
        app.field_rule = flags.appearance_preset.field_rule;
        app.tint = flags.appearance_preset.tint;
        app.subtitles = flags.appearance_preset.subtitles;
        app.started_at = flags.started_at;
        app.icon_lookup = flags.icon_lookup;
        app.link = flags.link;
        app.appearance = flags.appearance;
        app.appearance_link = flags.appearance_link;
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
            quick_launch: compass_core::config::DEFAULT_QUICK_LAUNCH,
            icons: compass_core::config::DEFAULT_ICONS,
            icon_lookup: IconLookup::default(),
            icon_cache: crate::icons::IconCache::new(),
            geometry: design::GEOMETRY,
            field_rule: false,
            tint: false,
            subtitles: true,
            first_frame_at: None,
            started_at: None,
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
    /// Split out from `LauncherApp::conceal` because the two outcomes it
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
        self.panel = None;
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

    /// One line saying what the launcher is showing, for a log.
    ///
    /// # Why this exists, and why it is a function rather than a `tracing!`
    ///
    /// The VM tier asserts what the launcher did by counting changed pixels.
    /// That is the only way to prove something reached the screen, and it is
    /// terrible at everything else: "5.44% of pixels in a box changed" cannot
    /// say *which* row is selected, what the query matched, or what the panel
    /// contains, and it moves with the font, the theme and the card geometry.
    /// Two of this tier's runs were spent on a containment box that had gone
    /// stale, asserting nothing about the launcher at all.
    ///
    /// So the state machine says what it did, in a line a grep can check
    /// exactly, and the pixels go back to proving only the thing they are
    /// uniquely good for: that a window was drawn.
    ///
    /// It is a pure function returning a `String` rather than a `tracing`
    /// macro at the call site so that its content is unit-testable. A log line
    /// asserted only inside a 25-minute VM run is a log line nobody can
    /// control.
    ///
    /// The shape is `key=value`, space separated, with titles quoted, because
    /// that is what survives being grepped out of a log interleaved with
    /// wgpu's debug output.
    #[must_use]
    pub fn state_line(&self) -> String {
        let mut line = format!(
            "query={:?} results={} selected={}",
            self.query,
            self.results.len(),
            self.selected
        );
        // The title, not just the index: an index is only meaningful against a
        // corpus the reader cannot see, and "selected=0" is equally true of a
        // right and a wrong first row.
        match self.selected_item() {
            Some(item) => line.push_str(&format!(" selected_title={:?}", item.name())),
            None => line.push_str(" selected_title=none"),
        }
        match &self.panel {
            None => line.push_str(" panel=closed"),
            Some(panel) => {
                line.push_str(&format!(
                    " panel=open panel_filter={:?} panel_rows={} panel_selected={}",
                    panel.filter,
                    panel.rows.len(),
                    panel.selected
                ));
                let title = usize::try_from(panel.selected)
                    .ok()
                    .and_then(|index| panel.rows.get(index))
                    .and_then(|row| {
                        let action = row.action?;
                        Some(
                            panel
                                .sections
                                .get(row.section)?
                                .actions
                                .get(action)?
                                .title
                                .as_str(),
                        )
                    });
                match title {
                    Some(title) => line.push_str(&format!(" panel_title={title:?}")),
                    None => line.push_str(" panel_title=none"),
                }
            }
        }
        if let Some(error) = &self.error {
            line.push_str(&format!(" error={error:?}"));
        }
        line.push_str(if self.window.is_some() {
            " window=open"
        } else {
            " window=hidden"
        });
        line
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
        // Only while the first frame is still owed. Once it has been reported
        // this stream is dropped, so the per-frame message stops entirely
        // rather than being produced and discarded at the refresh rate.
        if self.first_frame_at.is_none() {
            streams.push(window::frames().map(|_| Message::FrameDrawn));
        }
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
    ///
    /// Every message leaves a line in the log saying what the launcher now
    /// shows -- see [`LauncherApp::state_line`] for why. `debug`, not `info`:
    /// a line per keystroke is what makes a failure readable afterwards and is
    /// not what someone running the launcher wants in their terminal.
    pub fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.update_inner(message);
        tracing::debug!(target: "compass_ui::state", "{}", self.state_line());
        task
    }

    fn update_inner(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Initialize => Task::none(),

            // COLD START, RECORDED -- AND IT IS A LOWER BOUND, NOT THE SLA.
            //
            // §8.5 names "cold start to first frame < 120 ms" and nothing has
            // ever measured it. This is the first number.
            //
            // Iced yields this on `RedrawRequested`, which is the compositor
            // asking for a frame, NOT the frame reaching the screen. The
            // rendering that follows is unmeasured here and under llvmpipe it
            // is not small, so the figure is a floor on what a user waits for.
            // It is logged as `first_draw_ms` rather than `first_frame_ms` so
            // nobody reads it as the thing the SLA names.
            //
            // The elapsed time is from `AppFlags::started_at`, which `vicinae`
            // takes on entry -- so it excludes dynamic linking, and a wgpu
            // binary's is not free either. The VM tier measures from spawn to
            // this line and catches both.
            //
            // Recorded, not gated: ADR-0010. A threshold comes from the
            // numbers, once there are some.
            Message::FrameDrawn => {
                if self.first_frame_at.is_none() {
                    let now = std::time::Instant::now();
                    self.first_frame_at = Some(now);
                    if let Some(started) = self.started_at {
                        tracing::info!(
                            target: "compass_ui::startup",
                            first_draw_ms = now.duration_since(started).as_millis() as u64,
                            "first frame requested (a floor on cold start, not the §8.5 figure)"
                        );
                    }
                }
                Task::none()
            }
            Message::AppearanceChanged(appearance) => {
                self.appearance = appearance;
                Task::none()
            }
            Message::QueryChanged(query) => {
                self.panel = None;
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
                let action_id = item.action_id().map(str::to_owned);
                let launcher = Arc::clone(&self.launcher);
                launch_task(launcher, entry, action_id)
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
                    return focus_search();
                } else if let Some(item) = self.selected_item() {
                    // Only over a selected row. A panel of actions for nothing
                    // would be a panel whose every action fails.
                    self.panel = Some(PanelState::new(actions_for_app(item)));
                    return iced::widget::operation::focus(PANEL_INPUT);
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
            Message::PanelClicked(index) => {
                let Some(panel) = self.panel.as_mut() else {
                    return Task::none();
                };
                if !panel.rows.get(index).is_some_and(Row::selectable) {
                    return Task::none();
                }
                let Ok(selected) = isize::try_from(index) else {
                    return Task::none();
                };
                panel.selected = selected;
                self.update(Message::PanelActivate)
            }
            Message::PanelActivate => {
                let Some(panel) = self.panel.as_ref() else {
                    return Task::none();
                };
                let Some(action) = panel.selected_action() else {
                    return Task::none();
                };
                let Some(item) = self.selected_item() else {
                    return Task::none();
                };
                let task = match action.id.as_deref() {
                    Some(APP_OPEN) => {
                        self.panel = None;
                        return self.update(Message::LaunchSelected);
                    }
                    Some(APP_COPY_NAME) => iced::clipboard::write(item.name().to_owned()),
                    Some(APP_COPY_PATH) => {
                        let Some(path) = item.path() else {
                            return Task::none();
                        };
                        iced::clipboard::write(path.to_string_lossy().into_owned())
                    }
                    Some(id) if id.starts_with(APP_DESKTOP_ACTION) => {
                        let action_id = &id[APP_DESKTOP_ACTION.len()..];
                        if !item
                            .entry()
                            .actions()
                            .iter()
                            .any(|action| action.id() == action_id && action.exec().is_some())
                        {
                            return Task::none();
                        }
                        let task = launch_task(
                            Arc::clone(&self.launcher),
                            item.entry().clone(),
                            Some(action_id.to_owned()),
                        );
                        self.panel = None;
                        return task;
                    }
                    _ => return Task::none(),
                };
                self.panel = None;
                Task::batch([task, focus_search()])
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
                            return focus_search();
                        }
                        _ => {}
                    }
                    return match chord_direction(self.keybinding, key.as_ref(), modifiers) {
                        Some(direction) => self.update(Message::PanelMove(direction)),
                        None => Task::none(),
                    };
                }

                // Ctrl+1..9 launches the Nth row outright (#87). Above the
                // arrows because it is a complete action rather than a
                // movement, and below the panel branch because while the panel
                // is open the list is not what a keystroke is aimed at.
                //
                // OUT OF RANGE DOES NOTHING, deliberately. `max_results` can be
                // below nine, and a query can match two things; Ctrl+7 then
                // refers to no row. Launching the last one instead would be a
                // guess at what the user meant, and the thing it launches is
                // whatever happens to be at the bottom of an unrelated list.
                //
                // An out-of-range position falls through rather than returning
                // early. The early return was there to stop the chord reaching
                // the navigation matcher, and a control could not be made to
                // fire on it: no scheme maps a digit to a direction, so the two
                // paths are indistinguishable. Rather than keep a branch no
                // test can tell apart from its absence, it is gone -- and if a
                // scheme ever does bind digits, the test that catches it is the
                // one that notices the selection moving.
                if self.quick_launch
                    && let Some(position) = quick_launch_position(key.as_ref(), modifiers)
                    && position < self.results.len()
                {
                    self.selected = position;
                    return self.update(Message::LaunchSelected);
                }

                match key.as_ref() {
                    Key::Named(Named::ArrowDown) => {
                        return self.update(Message::MoveSelection(Direction::Down));
                    }
                    Key::Named(Named::ArrowUp) => {
                        return self.update(Message::MoveSelection(Direction::Up));
                    }
                    Key::Named(Named::Escape) => return self.update(Message::Dismiss),
                    Key::Named(Named::Enter) => return self.update(Message::LaunchSelected),
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
        let geometry = self.geometry;
        let palette = design::palette(self.appearance);

        let input = text_input("Search…", &self.query)
            .id(SEARCH_INPUT)
            .on_input_maybe(self.panel.is_none().then_some(Message::QueryChanged))
            .padding(Padding::new(0.0).left(14).right(14))
            .size(f32::from(geometry.query_size));

        let field = container(input)
            .height(Length::Fixed(f32::from(geometry.field_height)))
            .width(Length::Fill)
            .align_y(Alignment::Center)
            .style(move |_: &Theme| container::Style {
                background: Some(palette.field.to_iced().into()),
                border: Border {
                    color: palette.border.to_iced(),
                    width: 1.0,
                    radius: f32::from(geometry.field_radius).into(),
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
            let mut list = column![].spacing(f32::from(geometry.row_spacing));
            for (position, index) in self.results.iter().enumerate() {
                let Some(item) = self.app_index.items().get(*index) else {
                    continue;
                };
                list = list.push(self.result_row(item, position == self.selected));
            }
            container(list).padding(Padding::new(6.0).top(8)).into()
        };

        // Flow's hairline rule under the query field (#84). A one-pixel
        // container rather than a border on the field, because the field has
        // its own rounded border in the other presets and a rule has to span
        // the card's full width regardless of the field's radius.
        let card_content = if self.field_rule {
            column![
                field,
                container(Space::new())
                    .width(Length::Fill)
                    .height(Length::Fixed(1.0))
                    .style(move |_: &Theme| container::Style {
                        background: Some(palette.border.to_iced().into()),
                        ..container::Style::default()
                    }),
                body
            ]
            .width(Length::Fill)
        } else {
            column![field, body].width(Length::Fill)
        };

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

        let card_background = card_background(palette.surface, self.tint);

        container(
            container(card_body)
                .width(Length::Fixed(f32::from(geometry.card_width)))
                .padding(geometry.card_padding)
                .style(move |_: &Theme| container::Style {
                    background: Some(card_background.into()),
                    border: Border {
                        color: palette.border.to_iced(),
                        width: 1.0,
                        radius: f32::from(geometry.card_radius).into(),
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
        let geometry = self.geometry;
        let palette = design::palette(self.appearance);
        container(
            text(message.to_owned())
                .size(f32::from(geometry.title_size))
                .color(palette.muted.to_iced()),
        )
        .width(Length::Fill)
        .padding(28)
        .align_x(Alignment::Center)
        .into()
    }

    /// One result: icon, title, subtitle.
    ///
    /// The icon slot is a fixed square of `geometry.icon_size`, and what goes
    /// in it depends on `launcher.appearance.icons` (#85). Off, or on with a
    /// name the theme cannot resolve, it is the application's first letter in a
    /// tinted square. On and resolved, it is the icon itself.
    ///
    /// **The slot is the same size either way**, which is what keeps the
    /// fallback from being visible as a defect: a missing icon leaves the row's
    /// proportions and the title's position exactly where the others are, and
    /// the VM tier's window box does not move when the option is turned on.
    fn result_row(&self, item: &AppItem, selected: bool) -> Element<'_, Message> {
        let geometry = self.geometry;
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

        let icon: Element<Message> = match self.row_art(item) {
            Some(crate::icons::IconArt::Raster(path)) => {
                container(image(path).width(Length::Fill).height(Length::Fill))
                    .width(Length::Fixed(f32::from(geometry.icon_size)))
                    .height(Length::Fixed(f32::from(geometry.icon_size)))
                    .into()
            }
            Some(crate::icons::IconArt::Vector(path)) => {
                container(svg(path).width(Length::Fill).height(Length::Fill))
                    .width(Length::Fixed(f32::from(geometry.icon_size)))
                    .height(Length::Fixed(f32::from(geometry.icon_size)))
                    .into()
            }
            None => {
                let initial = item
                    .name()
                    .chars()
                    .next()
                    .map_or_else(String::new, |c| c.to_uppercase().to_string());

                container(
                    text(initial)
                        .size(f32::from(geometry.icon_size) / 2.0)
                        .color(title_color.to_iced()),
                )
                .width(Length::Fixed(f32::from(geometry.icon_size)))
                .height(Length::Fixed(f32::from(geometry.icon_size)))
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
                })
                .into()
            }
        };

        let mut labels = column![
            text(item.name().to_owned())
                .size(f32::from(geometry.title_size))
                .color(title_color.to_iced())
        ];
        // `subtitles` gates this, not just the presence of a comment: the dense
        // preset's row is one line tall and a second would overflow it.
        if self.subtitles
            && let Some(comment) = item.comment()
        {
            labels = labels.push(
                text(comment.to_owned())
                    .size(f32::from(geometry.subtitle_size))
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
        .height(Length::Fixed(f32::from(geometry.row_height)))
        .style(move |_: &Theme| {
            if selected {
                container::Style {
                    background: Some(palette.selection.to_iced().into()),
                    border: Border {
                        color: Color::TRANSPARENT,
                        width: 0.0,
                        radius: f32::from(geometry.row_radius).into(),
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
        let geometry = self.geometry;
        let palette = design::palette(self.appearance);
        let mut col = column![
            text_input("Search…", &panel.filter)
                .id(PANEL_INPUT)
                .on_input(Message::PanelFilterChanged)
                .padding(8)
                .size(f32::from(geometry.title_size))
        ]
        .spacing(f32::from(geometry.row_spacing));

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
                            .size(f32::from(geometry.heading_size))
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
                    mouse_area(self.panel_item(title, shortcut.as_deref(), selected))
                        .on_press(Message::PanelClicked(index))
                        .into()
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
        let geometry = self.geometry;
        let palette = design::palette(self.appearance);
        let colour = if selected {
            palette.selection_text
        } else {
            palette.text
        };

        let mut line = row![
            text(title.to_owned())
                .size(f32::from(geometry.title_size))
                .color(colour.to_iced())
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        if let Some(shortcut) = shortcut {
            line = line.push(Space::new().width(Length::Fill));
            line = line.push(
                text(shortcut.to_owned())
                    .size(f32::from(geometry.subtitle_size))
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

        self.results = self
            .app_index
            .search_root(&self.query, None)
            .into_iter()
            .map(|scored| scored.index)
            .collect();
        // Back to the top on every new query: the old selection pointed into a
        // different list, and keeping its position would silently select an
        // unrelated application.
        self.selected = 0;
        self.warm_icons();
    }

    /// What goes in a row's icon slot: resolved art, or nothing for the initial.
    ///
    /// The three ways to get nothing -- icons off, an entry with no `Icon=`,
    /// and a name the theme could not resolve -- are deliberately one answer.
    /// They all mean the same thing to the row, and collapsing them here is
    /// what keeps `view` from having to know the difference.
    fn row_art(&self, item: &AppItem) -> Option<&crate::icons::IconArt> {
        if !self.icons {
            return None;
        }
        item.icon().and_then(|name| self.icon_cache.cached(name))
    }

    /// Resolve the icons of the rows now on screen.
    ///
    /// Here rather than in `view` because resolution walks the theme
    /// directories, and a launcher re-ranks on every keystroke: doing disk work
    /// per draw is the cost #85 warns about. Bounded to the current results, so
    /// it is the size of the visible list rather than of the index, and the
    /// cache makes each name at most one walk for the life of the process.
    ///
    /// A no-op when icons are off, so the default configuration does no lookup
    /// at all.
    fn warm_icons(&mut self) {
        if !self.icons {
            return;
        }

        let names: Vec<&str> = self
            .results
            .iter()
            .filter_map(|index| self.app_index.items().get(*index))
            .filter_map(AppItem::icon)
            .collect();
        let find = self.icon_lookup.clone();
        self.icon_cache.warm(names, &|name| find.find(name));
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

    fn clipboard_writes(task: Task<Message>) -> Vec<String> {
        use iced::futures::{StreamExt, executor::block_on};
        use iced_winit::runtime::{Action, clipboard, task};

        let Some(stream) = task::into_stream(task) else {
            return Vec::new();
        };
        block_on(
            stream
                .filter_map(|action| async move {
                    match action {
                        Action::Clipboard(clipboard::Action::Write { contents, .. }) => {
                            Some(contents)
                        }
                        Action::Widget(_) => None,
                        other => panic!("copy issued an unexpected action: {other:?}"),
                    }
                })
                .collect(),
        )
    }

    #[derive(Debug, Default)]
    struct RecordingLaunchTarget(std::sync::Mutex<Vec<Option<String>>>);

    impl AppLauncher for RecordingLaunchTarget {
        fn launch<'a>(
            &'a self,
            _entry: &'a compass_xdg::DesktopEntry,
            _uris: &'a [&'a str],
        ) -> compass_platform::LaunchFuture<'a> {
            Box::pin(async move {
                self.0.lock().unwrap().push(None);
                Ok(compass_platform::LaunchMethod::Direct)
            })
        }

        fn launch_action<'a>(
            &'a self,
            _entry: &'a compass_xdg::DesktopEntry,
            action_id: &'a str,
            _uris: &'a [&'a str],
        ) -> compass_platform::LaunchFuture<'a> {
            Box::pin(async move {
                self.0.lock().unwrap().push(Some(action_id.to_owned()));
                Ok(compass_platform::LaunchMethod::Direct)
            })
        }
    }

    fn recorded_launch(action: bool) -> Vec<Option<String>> {
        use iced::futures::{StreamExt, executor::block_on};
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("browser.desktop"), "[Desktop Entry]\nType=Application\nName=Browser\nExec=parent\nActions=private;\n[Desktop Action private]\nName=Private Window\nExec=private-app\n").unwrap();
        let index = AppIndex::builder().dir(dir.path()).build();
        let selected = index
            .items()
            .iter()
            .position(|item| item.is_action() == action)
            .unwrap();
        let launcher = Arc::new(RecordingLaunchTarget::default());
        let mut app = LauncherApp::with_index(index).with_launcher(launcher.clone());
        app.results = vec![selected];
        let task = app.update(Message::LaunchSelected);
        let stream = iced_winit::runtime::task::into_stream(task).expect("launch task");
        let _outputs = block_on(stream.collect::<Vec<_>>());
        launcher.0.lock().unwrap().clone()
    }

    #[test]
    fn selecting_a_desktop_action_dispatches_its_id_not_the_parent() {
        assert_eq!(recorded_launch(true), [Some("private".to_owned())]);
    }

    #[test]
    fn selecting_an_application_still_dispatches_the_parent() {
        assert_eq!(recorded_launch(false), [None]);
    }

    #[test]
    fn a_root_application_exposes_and_dispatches_its_desktop_action_in_the_panel() {
        use iced::futures::{StreamExt, executor::block_on};
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("browser.desktop"), "[Desktop Entry]\nType=Application\nName=Browser\nExec=parent\nActions=private;\n[Desktop Action private]\nName=Private Window\nExec=private-app\n").unwrap();
        let index = AppIndex::builder().dir(dir.path()).build();
        let launcher = Arc::new(RecordingLaunchTarget::default());
        let mut app = LauncherApp::with_index(index).with_launcher(launcher.clone());
        let _ = app.update(Message::QueryChanged("Browser".to_owned()));
        assert_eq!(app.results.len(), 1, "actions are not duplicate root rows");
        let _ = app.update(Message::TogglePanel);
        let _ = app.update(Message::PanelFilterChanged("Private".to_owned()));
        assert_eq!(
            app.panel
                .as_ref()
                .unwrap()
                .selected_action()
                .unwrap()
                .id
                .as_deref(),
            Some("app.desktop:private")
        );
        let task = app.update(Message::PanelActivate);
        assert!(app.panel.is_none());
        let stream = iced_winit::runtime::task::into_stream(task).unwrap();
        let _ = block_on(stream.collect::<Vec<_>>());
        assert_eq!(*launcher.0.lock().unwrap(), [Some("private".to_owned())]);
    }

    #[test]
    fn filtered_copy_actions_write_the_selected_apps_name_and_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let expected_name = app.selected_item().expect("selected").name().to_owned();
        let expected_path = app
            .selected_item()
            .expect("selected")
            .path()
            .expect("desktop path")
            .to_string_lossy()
            .into_owned();
        for (filter, expected) in [("name", expected_name), ("path", expected_path)] {
            let _ = app.update(Message::TogglePanel);
            let _ = app.update(Message::PanelFilterChanged(filter.to_owned()));
            let task = app.update(pressed(iced::keyboard::key::Named::Enter));
            assert_eq!(clipboard_writes(task), vec![expected]);
            assert!(app.panel.is_none());
            assert_eq!(app.query, "fi");
        }
    }

    #[test]
    fn dispatch_uses_identity_even_when_the_label_changes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let expected = app.selected_item().expect("selected").name().to_owned();
        let _ = app.update(Message::TogglePanel);
        let panel = app.panel.as_mut().expect("panel");
        panel.sections[1].actions[0].title = "Copier le nom".to_owned();
        panel.set_filter("Copier le nom".to_owned());
        let task = app.update(Message::PanelActivate);
        assert_eq!(clipboard_writes(task), vec![expected]);
    }

    #[test]
    fn an_empty_filter_result_cannot_launch_or_copy() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let _ = app.update(Message::TogglePanel);
        let _ = app.update(Message::PanelFilterChanged("zzzznotanaction".to_owned()));
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        assert!(iced_winit::runtime::task::into_stream(task).is_none());
        assert!(app.panel.is_some());
    }

    #[test]
    fn changing_the_root_query_invalidates_its_panel() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let _ = app.update(Message::TogglePanel);
        let _ = app.update(Message::QueryChanged("terminal".to_owned()));
        assert!(app.panel.is_none());
        assert_eq!(app.selected_item().expect("selected").name(), "Terminal");
        assert!(clipboard_writes(app.update(Message::PanelActivate)).is_empty());
    }

    #[test]
    fn clicking_an_action_copies_but_clicking_a_heading_does_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let expected = app.selected_item().expect("selected").name().to_owned();
        let _ = app.update(Message::TogglePanel);
        assert!(clipboard_writes(app.update(Message::PanelClicked(2))).is_empty());
        assert!(app.panel.is_some());
        let messages = {
            let mut ui = iced_test::simulator(app.view());
            ui.click("Copy name").expect("click the action");
            ui.into_messages().collect::<Vec<_>>()
        };
        assert_eq!(messages.len(), 1);
        let writes: Vec<_> = messages
            .into_iter()
            .flat_map(|message| clipboard_writes(app.update(message)))
            .collect();
        assert_eq!(writes, vec![expected]);
    }

    #[test]
    fn typing_in_the_panel_emits_only_panel_edits_and_no_submit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let _ = app.update(Message::TogglePanel);
        let mut ui = iced_test::simulator(app.view());
        ui.click(iced_test::selector::id(PANEL_INPUT))
            .expect("panel input");
        ui.typewrite("n");
        ui.tap_key(iced::keyboard::key::Named::Enter);
        let messages: Vec<_> = ui.into_messages().collect();
        assert!(
            matches!(messages.as_slice(), [Message::PanelFilterChanged(filter)] if filter == "n")
        );
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

    /// The log every test in this binary writes into, installed once.
    ///
    /// A thread-local `with_default` subscriber was the obvious way to do this
    /// and does not work here: `tracing` keeps a PROCESS-GLOBAL max-level
    /// hint, so with tests running in parallel a `debug!` can be filtered out
    /// while a thread-local DEBUG subscriber is active. The test passed alone
    /// and failed in the suite -- 38/38 pass under `--test-threads=1`, which
    /// is how that was pinned down rather than guessed.
    ///
    /// Installing one global subscriber at DEBUG raises that hint for the
    /// whole run and keeps it raised, so there is no race to lose.
    fn log() -> &'static std::sync::Mutex<Vec<u8>> {
        use std::sync::{Mutex, OnceLock};
        use tracing_subscriber::fmt::MakeWriter;

        static LOG: OnceLock<&'static Mutex<Vec<u8>>> = OnceLock::new();

        #[derive(Clone, Copy)]
        struct Writer(&'static Mutex<Vec<u8>>);

        impl std::io::Write for Writer {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        impl<'a> MakeWriter<'a> for Writer {
            type Writer = Self;
            fn make_writer(&'a self) -> Self::Writer {
                *self
            }
        }

        LOG.get_or_init(|| {
            let buffer: &'static Mutex<Vec<u8>> = Box::leak(Box::new(Mutex::new(Vec::new())));
            let subscriber = tracing_subscriber::fmt()
                .with_max_level(tracing::Level::DEBUG)
                .with_writer(Writer(buffer))
                .without_time()
                .finish();
            // Another test binary in the same process may have got here first;
            // either way a DEBUG subscriber is installed, which is what this
            // needs.
            let _ = tracing::subscriber::set_global_default(subscriber);
            buffer
        })
    }

    fn logged() -> String {
        let bytes = log()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[test]
    fn every_message_leaves_its_state_in_the_log() {
        // A query no other test uses, so the lines below are this test's even
        // though every test in the binary shares one log. Counting lines that
        // merely say `compass_ui::state` would race with whatever else is
        // running.
        const QUERY: &str = "compass-log-state-sentinel";

        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("log-state.desktop"),
            format!("[Desktop Entry]\nType=Application\nName={QUERY}\nExec=true\n"),
        )
        .expect("log-state fixture");
        let _ = log();
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged(QUERY.to_owned()));
        let _ = app.update(Message::TogglePanel);

        let log = logged();
        let mine: Vec<&str> = log
            .lines()
            .filter(|line| line.contains(&format!(r#"query="{QUERY}""#)))
            .collect();

        // If `update` stops emitting, this fails here in milliseconds instead
        // of in a 25-minute VM run against a line the launcher no longer
        // writes. A control confirmed the other state-line tests do NOT catch
        // that: deleting the `tracing::debug!` left all of them green.
        assert_eq!(
            mine.len(),
            2,
            "one line per message, no more and no fewer:\n{log}"
        );
        assert!(mine[0].contains("compass_ui::state"), "{:?}", mine[0]);
        assert!(mine[0].contains("panel=closed"), "{:?}", mine[0]);
        assert!(mine[1].contains("panel=open"), "{:?}", mine[1]);
    }

    #[test]
    fn the_state_line_names_what_is_on_screen() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());

        let _ = app.update(Message::QueryChanged("fir".to_owned()));
        let line = app.state_line();
        // Everything the VM tier wants to assert, in one greppable line.
        assert!(line.contains(r#"query="fir""#), "{line}");
        assert!(line.contains("results=1"), "{line}");
        assert!(line.contains("selected=0"), "{line}");
        assert!(line.contains(r#"selected_title="Firefox""#), "{line}");
        assert!(line.contains("panel=closed"), "{line}");
    }

    #[test]
    fn the_state_line_follows_the_selection_rather_than_the_index_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());

        // A query matching more than one row, so moving down has somewhere to
        // go. Not the empty query: the launcher shows nothing for that, which
        // this test discovered. The title is what makes "selected=1" mean
        // anything.
        let _ = app.update(Message::QueryChanged("fi".to_owned()));
        let first = app.state_line();
        let _ = app.update(Message::MoveSelection(Direction::Down));
        let second = app.state_line();

        assert!(first.contains("selected=0"), "{first}");
        assert!(second.contains("selected=1"), "{second}");
        assert_ne!(
            first
                .split_once("selected_title=")
                .map(|(_, rest)| rest.to_owned()),
            second
                .split_once("selected_title=")
                .map(|(_, rest)| rest.to_owned()),
            "the title must move with the selection, not just the number"
        );
    }

    #[test]
    fn the_state_line_describes_the_panel_when_it_is_open() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("fir".to_owned()));

        assert!(app.state_line().contains("panel=closed"));

        let _ = app.update(Message::TogglePanel);
        let line = app.state_line();
        assert!(line.contains("panel=open"), "{line}");
        assert!(line.contains("panel_selected=0"), "{line}");
        // The first SELECTABLE row, which is the thing the VM frame had to be
        // read by eye to confirm. Now it is a string.
        assert!(line.contains(r#"panel_title="Open""#), "{line}");

        let _ = app.update(Message::PanelFilterChanged("copy".to_owned()));
        let filtered = app.state_line();
        assert!(filtered.contains(r#"panel_filter="copy""#), "{filtered}");
        assert!(filtered.contains(r#"panel_title="Copy"#), "{filtered}");

        let _ = app.update(Message::TogglePanel);
        assert!(app.state_line().contains("panel=closed"));
    }

    #[test]
    fn the_state_line_says_when_nothing_matched() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("zzzz".to_owned()));
        let line = app.state_line();
        // "results=0" and "no window" are different failures and the tier has
        // to be able to tell them apart from a log.
        assert!(line.contains("results=0"), "{line}");
        assert!(line.contains("selected_title=none"), "{line}");
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
    /// A `Ctrl`+`digit` press. `chord` below builds it; this names the intent.
    pub(super) fn ctrl_digit(digit: &str) -> Message {
        chord(digit, iced::keyboard::Modifiers::CTRL)
    }

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
    pub(super) fn app_with_rows(dir: &std::path::Path) -> LauncherApp {
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

#[cfg(test)]
mod quick_launch_tests {
    use super::*;
    use iced::keyboard::{Key, Modifiers};

    #[test]
    fn ctrl_one_through_nine_are_positions_counting_from_zero() {
        for (digit, want) in [("1", 0), ("2", 1), ("5", 4), ("9", 8)] {
            assert_eq!(
                quick_launch_position(Key::Character(digit), Modifiers::CTRL),
                Some(want),
                "Ctrl+{digit}"
            );
        }
    }

    #[test]
    fn there_is_no_ctrl_zero() {
        // The digits are one-based because the rows are. A zero would have to
        // mean either "the first" or "the tenth" and neither is what anyone
        // presses it for.
        assert_eq!(
            quick_launch_position(Key::Character("0"), Modifiers::CTRL),
            None
        );
    }

    #[test]
    fn a_digit_without_control_is_somebody_elses() {
        // Typing "3" into the search field must reach the field.
        assert_eq!(
            quick_launch_position(Key::Character("3"), Modifiers::default()),
            None
        );
    }

    #[test]
    fn control_plus_another_modifier_is_a_different_chord() {
        // Not ours to claim: the desktop may already bind these, and on many
        // layouts Ctrl+Shift+1 is not a digit at all.
        for extra in [Modifiers::SHIFT, Modifiers::ALT, Modifiers::LOGO] {
            assert_eq!(
                quick_launch_position(Key::Character("1"), Modifiers::CTRL | extra),
                None,
                "{extra:?}"
            );
        }
    }

    #[test]
    fn a_non_digit_character_is_not_a_quick_launch() {
        for text in ["b", "-", "", "1x", "12"] {
            assert_eq!(
                quick_launch_position(Key::Character(text), Modifiers::CTRL),
                None,
                "{text:?}"
            );
        }
    }

    #[test]
    fn ctrl_n_selects_and_launches_the_nth_visible_row() {
        // THE BUG THIS IS HERE FOR: `results` holds indices into the
        // application index, and the selection is a position within `results`.
        // Ctrl+3 means the third ROW, not item 3. Confusing the two launches
        // something plausible and wrong, which is the worst kind to have here
        // because nothing looks broken.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = super::tests::app_with_rows(dir.path());

        let rows: Vec<String> = app
            .results
            .iter()
            .filter_map(|i| app.app_index.items().get(*i))
            .map(|item| item.name().to_owned())
            .collect();
        assert!(rows.len() >= 2, "need several rows: {rows:?}");

        for (position, name) in rows.iter().enumerate().take(9) {
            let digit = (position + 1).to_string();
            let _ = app.update(super::tests::ctrl_digit(&digit));
            assert_eq!(
                app.selected_item().map(compass_core::AppItem::name),
                Some(name.as_str()),
                "Ctrl+{digit} must resolve to row {position}"
            );
        }
    }

    #[test]
    fn a_row_that_is_not_there_does_nothing_at_all() {
        // THE BOUNDS ARE THE ROWS', NOT THE INDEX'S. A query filters the list,
        // so `results` is shorter than `app_index.items()` -- and a check
        // against the wrong one accepts a position that has no row, then
        // selects it. The fixture is queried precisely so the two lengths
        // differ: a control that swapped them was SILENT until this test did.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = super::tests::app_with_rows(dir.path());
        let _ = app.update(Message::QueryChanged("fir".to_owned()));

        let rows = app.results.len();
        let items = app.app_index.items().len();
        assert!(
            rows < items,
            "the query must narrow the list: {rows} of {items}"
        );
        assert!(rows < 9, "and leave fewer than nine rows: {rows}");

        let before = app.selected;
        // A position with no row, but well inside the item index.
        let _ = app.update(super::tests::ctrl_digit(&(rows + 1).to_string()));
        assert_eq!(app.selected, before, "the selection must not move either");
    }

    #[test]
    fn the_setting_turns_it_off() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = super::tests::app_with_rows(dir.path());
        app.quick_launch = false;
        app.selected = 1;

        let _ = app.update(super::tests::ctrl_digit("1"));
        assert_eq!(
            app.selected, 1,
            "with quick_launch off, Ctrl+1 must not move the selection"
        );
    }

    #[test]
    fn it_is_on_by_default() {
        // #87 asks for on by default: it costs nothing when unused, because
        // the chords are otherwise unbound.
        assert!(compass_core::config::LauncherConfig::default().quick_launch());
        assert!(AppFlags::default().quick_launch);
    }

    #[test]
    fn the_panel_takes_the_chord_while_it_is_open() {
        // While the panel is open the list is not what a keystroke is aimed
        // at, and launching a row out from under an open action panel would be
        // a surprise.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = super::tests::app_with_rows(dir.path());
        let _ = app.update(Message::TogglePanel);
        assert!(app.panel.is_some(), "precondition");
        app.selected = 1;

        let _ = app.update(super::tests::ctrl_digit("1"));
        assert_eq!(app.selected, 1, "the row under the panel did not move");
        assert!(app.panel.is_some(), "and the panel is still open");
    }

    #[test]
    fn a_named_key_is_not_a_quick_launch() {
        assert_eq!(
            quick_launch_position(
                Key::Named(iced::keyboard::key::Named::Enter),
                Modifiers::CTRL
            ),
            None
        );
    }
}

#[cfg(test)]
mod icon_tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::icons::IconArt;

    /// Three entries, two of which name an icon.
    fn app_with_icons(dir: &std::path::Path) -> LauncherApp {
        for (file, name, icon) in [
            ("firefox.desktop", "Firefox", Some("firefox")),
            ("files.desktop", "Files", Some("system-file-manager")),
            ("terminal.desktop", "Terminal", None),
        ] {
            let icon_line = icon.map_or_else(String::new, |value| format!("Icon={value}\n"));
            fs::write(
                dir.join(file),
                format!(
                    "[Desktop Entry]\nType=Application\nName={name}\nExec=/bin/true\n{icon_line}"
                ),
            )
            .expect("write entry");
        }
        LauncherApp::with_index(AppIndex::builder().dir(dir).build())
    }

    /// A lookup that answers for `hits` and records every name it is asked.
    fn recording(hits: &[&str]) -> (Arc<Mutex<Vec<String>>>, IconLookup) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&asked);
        let hits: Vec<String> = hits.iter().map(|hit| (*hit).to_owned()).collect();
        let lookup = IconLookup::new(move |name: &str| {
            log.lock().expect("lock").push(name.to_owned());
            hits.iter()
                .any(|hit| hit == name)
                .then(|| PathBuf::from(format!("/i/{name}.svg")))
        });
        (asked, lookup)
    }

    fn asked(log: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
        log.lock().expect("lock").clone()
    }

    fn row<'a>(app: &'a LauncherApp, name: &str) -> &'a AppItem {
        app.results
            .iter()
            .filter_map(|index| app.app_index.items().get(*index))
            .find(|item| item.name() == name)
            .unwrap_or_else(|| panic!("{name} is not a row"))
    }

    #[test]
    fn icons_off_looks_nothing_up_at_all() {
        // The default configuration must cost nothing: not a directory walk,
        // not even a cached miss.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_icons(dir.path());
        assert!(!app.icons, "precondition: off by default");

        let (log, lookup) = recording(&["firefox"]);
        app.icon_lookup = lookup;
        let _ = app.update(Message::QueryChanged("fi".to_owned()));

        assert!(!app.results.is_empty(), "precondition: there are rows");
        assert!(asked(&log).is_empty(), "{:?}", asked(&log));
        assert!(app.icon_cache.is_empty());
    }

    #[test]
    fn icons_on_warms_only_the_rows_on_screen() {
        // Bounded by the visible list rather than by the index: that is the
        // whole reason resolution happens in `update` and not in `view`.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_icons(dir.path());
        app.icons = true;

        let (log, lookup) = recording(&["firefox"]);
        app.icon_lookup = lookup;
        let _ = app.update(Message::QueryChanged("firefox".to_owned()));

        assert_eq!(asked(&log), ["firefox"], "{:?}", asked(&log));
    }

    #[test]
    fn an_entry_with_no_icon_key_is_never_looked_up() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_icons(dir.path());
        app.icons = true;

        let (log, lookup) = recording(&[]);
        app.icon_lookup = lookup;
        let _ = app.update(Message::QueryChanged("terminal".to_owned()));

        assert!(!app.results.is_empty(), "precondition: Terminal is a row");
        assert!(asked(&log).is_empty(), "{:?}", asked(&log));
    }

    #[test]
    fn the_same_name_is_resolved_once_however_many_keystrokes() {
        // A launcher re-ranks on every keystroke. Without the cache this is
        // one theme walk per character typed.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_icons(dir.path());
        app.icons = true;

        let (log, lookup) = recording(&["firefox"]);
        app.icon_lookup = lookup;
        for query in ["f", "fi", "fir", "fire", "firef"] {
            let _ = app.update(Message::QueryChanged(query.to_owned()));
        }

        let names = asked(&log);
        assert_eq!(
            names.iter().filter(|name| *name == "firefox").count(),
            1,
            "{names:?}"
        );
    }

    #[test]
    fn a_resolved_row_draws_its_icon_and_an_unresolved_one_falls_back() {
        // The ordinary state of a mixed desktop: the theme has one of them.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_icons(dir.path());
        app.icons = true;

        let (_log, lookup) = recording(&["firefox"]);
        app.icon_lookup = lookup;
        let _ = app.update(Message::QueryChanged("f".to_owned()));

        assert_eq!(
            app.row_art(row(&app, "Firefox")),
            Some(&IconArt::Vector(PathBuf::from("/i/firefox.svg")))
        );
        assert_eq!(
            app.row_art(row(&app, "Files")),
            None,
            "an icon the theme does not have must fall back to the initial"
        );
    }

    #[test]
    fn a_resolved_icon_is_still_not_drawn_once_icons_are_turned_off() {
        // The cache outlives the toggle, so the gate has to be at the draw and
        // not only at the warm.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_icons(dir.path());
        app.icons = true;

        let (_log, lookup) = recording(&["firefox"]);
        app.icon_lookup = lookup;
        let _ = app.update(Message::QueryChanged("firefox".to_owned()));

        assert!(
            app.row_art(row(&app, "Firefox")).is_some(),
            "precondition: resolved"
        );

        app.icons = false;
        assert_eq!(app.row_art(row(&app, "Firefox")), None);
    }

    #[test]
    fn it_is_off_by_default() {
        // #85 asks for off by default: the default look is Spotlight-simple,
        // and icons are what make it busier.
        assert!(
            !compass_core::config::LauncherConfig::default()
                .appearance()
                .icons()
        );
        assert!(!AppFlags::default().icons);
    }
}

/// The `tint` option (#86): translucency, and deliberately not called blur.
#[cfg(test)]
mod tint_tests {
    use super::*;
    use crate::preset::{self, Preset};

    #[test]
    fn only_raycast_tints() {
        // Raycast is the one preset imitating a look built on real compositor
        // blur, so it gets the closest thing available. The Spotlight-simple
        // default does not, which is also what keeps the VM tier's pixel gates
        // seeing an unchanged image.
        assert!(Preset::Raycast.tint());
        assert!(!Preset::Gnome.tint());
        assert!(!Preset::Flow.tint());
        assert!(!Preset::Rofi.tint());
    }

    #[test]
    fn an_explicit_setting_beats_the_preset_both_ways() {
        // Both directions: a one-way override reads as working until someone
        // tries to turn the feature off under a preset that enables it.
        assert!(!preset::resolve(Some("raycast"), None, Some(false)).tint);
        assert!(preset::resolve(Some("gnome"), None, Some(true)).tint);
    }

    #[test]
    fn the_flag_reaches_the_launcher() {
        // Through `apply`, which is the real assignment list `vicinae::run`
        // uses -- a helper that set the field itself would be a test of the
        // helper. This is the control #108 recorded and could not close.
        let mut app = LauncherApp::with_index(AppIndex::default());
        app.apply(AppFlags {
            appearance_preset: preset::resolve(Some("raycast"), None, None),
            ..AppFlags::default()
        });
        assert!(app.tint, "the resolved preset's tint never reached the app");

        app.apply(AppFlags {
            appearance_preset: preset::resolve(Some("gnome"), None, None),
            ..AppFlags::default()
        });
        assert!(!app.tint);
    }

    #[test]
    fn tint_changes_the_card_background_and_only_its_alpha() {
        let surface = design::DARK.surface;
        let opaque = card_background(surface, false);
        let tinted = card_background(surface, true);

        assert!(
            (opaque.a - 1.0).abs() < f32::EPSILON,
            "the untinted card must be fully opaque, got alpha {}",
            opaque.a
        );
        assert!(
            tinted.a < opaque.a,
            "tint did not make the card more transparent: {} vs {}",
            tinted.a,
            opaque.a
        );

        // Translucency, not a different colour. If the hue moved, this would be
        // a theme change wearing a transparency setting's name.
        assert!(
            (tinted.r - opaque.r).abs() < f32::EPSILON
                && (tinted.g - opaque.g).abs() < f32::EPSILON
                && (tinted.b - opaque.b).abs() < f32::EPSILON,
            "tint changed the card's colour, not just its alpha"
        );
    }

    #[test]
    fn the_tinted_card_stays_legible() {
        // A launcher bound to Super+Space is text over an arbitrary wallpaper.
        // There is a temptation to push the alpha down because it looks better
        // in a screenshot over a photo; this is the floor that stops it.
        //
        // Asserted on the colour the view actually fills with, not on
        // `TINT_ALPHA` directly: clippy rejects an assertion over a constant,
        // and rightly -- `assert!(SOME_CONST >= 0.75)` is evaluated by the
        // compiler, not by the test, so it proves nothing about the code path.
        // The same mistake was already fixed once here in a quick-launch
        // assertion over `DEFAULT_QUICK_LAUNCH`.
        let alpha = card_background(design::DARK.surface, true).a;
        assert!(
            alpha >= 0.75,
            "the tinted card fills at alpha {alpha}, low enough that body text over a bright \
             wallpaper loses contrast. Legibility is not negotiable for the launcher."
        );
        assert!(
            alpha < 1.0,
            "a tint that fills at alpha {alpha} is not a tint"
        );
    }
}

#[cfg(test)]
mod preset_tests {
    use super::*;
    use crate::preset::{self, Preset};

    /// A launcher built from flags, through the code `vicinae` uses.
    ///
    /// `apply` rather than a hand-written assignment list: a helper that set
    /// the fields itself would pass while the real copying silently stopped,
    /// which is the whole mutation these tests exist to catch. It was written
    /// that way first and the control caught it.
    fn app_with(preset_name: &str, icons: Option<bool>) -> LauncherApp {
        let resolved = preset::resolve(Some(preset_name), icons, None);
        let mut app = LauncherApp::with_index(AppIndex::builder().build());
        app.apply(AppFlags {
            icons: resolved.icons,
            appearance_preset: resolved,
            ..AppFlags::default()
        });
        app
    }

    #[test]
    fn the_preset_reaches_the_state_that_draws() {
        // The wiring, not the resolver: `preset::resolve` is tested on its
        // own, and this is the half that would silently do nothing if the
        // flags stopped being copied across.
        let rofi = app_with("rofi", None);
        assert_eq!(rofi.geometry.row_height, Preset::Rofi.geometry().row_height);
        assert_eq!(rofi.geometry.card_radius, 0);
        assert!(!rofi.icons);
        assert!(!rofi.field_rule);
        assert!(!rofi.subtitles, "the dense preset draws one line per row");

        let flow = app_with("flow", None);
        assert!(flow.field_rule, "flow draws the rule under the field");
        assert!(flow.icons);
    }

    #[test]
    fn the_default_launcher_draws_the_shipped_geometry() {
        // `with_index` is what every other test builds, so a preset layer that
        // changed the default would change all of them -- and move the VM
        // tier's containment box with no commit saying so.
        let app = LauncherApp::with_index(AppIndex::builder().build());
        assert_eq!(
            format!("{:?}", app.geometry),
            format!("{:?}", design::GEOMETRY)
        );
        assert!(!app.field_rule);
    }

    #[test]
    fn a_preset_and_an_explicit_key_both_reach_the_app() {
        // raycast turns icons on; the explicit key turns them back off.
        assert!(app_with("raycast", None).icons);
        assert!(!app_with("raycast", Some(false)).icons);
    }
}

/// Tests that render `view` headlessly.
///
/// # Why this module exists
///
/// Every other test in this crate asserts on state. `view` returns an opaque
/// `Element`, so a branch inside it could be deleted with nothing failing --
/// and one was: #108 recorded a control that mutated away the `field_rule`
/// branch and left all 79 tests green. The design surrogate did not close it
/// either, because it builds its own DOM from the same tokens and never
/// executes the Iced tree.
///
/// `iced_test` renders the real tree in headless mode, which is what finally
/// makes those branches reachable from a test.
///
/// # Bounds rather than pixels
///
/// `Simulator::snapshot` compares against a committed PNG. That would make the
/// assertion depend on which fonts the machine running it happens to have, and
/// a gate that reddens when a runner image changes its font package is a gate
/// people turn off. Selecting a widget by its text and reading its **layout
/// bounds** asserts the same structure without depending on how the glyphs came
/// out.
/// Input-method behaviour: the "watch for" item on the Phase 1 issue.
///
/// #4 flags IME and screen-reader behaviour with an explicit deadline: "Test
/// both here, not in Phase 5 — if Iced can't do them, ADR-0001 needs
/// revisiting while that is still cheap." Nothing tested it, so the risk was
/// carried unresolved rather than answered.
///
/// # The stack does support it, end to end
///
/// * `winit` 0.30 implements `zwp_text_input_v3` on Wayland
///   (`platform_impl/linux/wayland/seat/text_input/`) and emits
///   `WindowEvent::Ime(Enabled | Preedit | Commit | Disabled)`.
/// * `iced_winit` 0.14 converts those into `Event::InputMethod`
///   (`conversion.rs`), and calls `set_ime_allowed`, `set_ime_cursor_area` and
///   `set_ime_purpose` from `enable_ime`, which runs when a widget asks for an
///   input method — so a focused search field turns the IME on by itself.
/// * `iced_core` carries `InputMethod` and `Preedit`.
///
/// So **ADR-0001 does not need revisiting on this point.** That is a claim
/// about libraries, and libraries change, which is what these tests are for:
/// they fail if a future Iced stops routing composed text to the field.
///
/// # What these do and do not prove
///
/// They drive `Event::InputMethod` through the real widget tree and assert the
/// query is what a CJK or accented commit should leave behind. They do **not**
/// prove a real IME works against a real compositor — that needs ibus or fcitx
/// in the VM tier, and it is a different test. What they rule out is the
/// cheaper and more likely failure: composed text never reaching the field at
/// all, which would make the launcher unusable for anyone typing Japanese,
/// Chinese, Korean or with a compose key, and which no other test would catch.
#[cfg(test)]
mod ime_tests {
    use super::*;
    use iced_winit::core::input_method;

    fn focused_app() -> LauncherApp {
        let mut app = LauncherApp::with_index(AppIndex::default());
        app.apply(AppFlags::default());
        app
    }

    /// One simulator, focused, driven by `events`, with the messages applied.
    ///
    /// A fresh simulator per interaction would rebuild the widget tree and lose
    /// focus, so a two-step sequence has to share one. Focus itself comes from
    /// clicking the placeholder: in the running launcher it arrives via a
    /// `Task` (`focus_search`, dispatched on show) and the simulator does not
    /// run tasks.
    fn drive(app: &mut LauncherApp, events: Vec<Vec<iced_winit::core::Event>>) {
        let mut out = Vec::new();
        for batch in events {
            // Scoped so the borrow of `app.view()` ends before `app.update`.
            {
                let mut ui = iced_test::simulator(app.view());
                ui.click("Search…").expect("the search field is clickable");
                let _ = ui.simulate(batch);
                out.extend(ui.into_messages());
            }
            for message in out.drain(..) {
                let _ = app.update(message);
            }
        }
    }

    fn commit(text: &str) -> Vec<iced_winit::core::Event> {
        vec![iced_winit::core::Event::InputMethod(
            input_method::Event::Commit(text.to_owned()),
        )]
    }

    /// CONTROL. If plain typing does not reach the query in this harness, the
    /// IME assertions below are measuring the harness, not the input method.
    /// This failed first — the field was unfocused and nothing reached it —
    /// which is exactly the false "IME is broken" this control exists to stop.
    #[test]
    fn control_typewrite_reaches_the_query() {
        let mut app = focused_app();
        let mut ui = iced_test::simulator(app.view());
        ui.click("Search…").expect("the search field is clickable");
        let _ = ui.typewrite("abc");
        for message in ui.into_messages() {
            let _ = app.update(message);
        }
        assert_eq!(
            app.query, "abc",
            "CONTROL: plain typing did not reach the query, so nothing below measures IME"
        );
    }

    #[test]
    fn a_committed_composition_reaches_the_query() {
        let mut app = focused_app();
        drive(&mut app, vec![commit("日本語")]);
        assert_eq!(
            app.query, "日本語",
            "a committed IME composition did not reach the query field. Typed ASCII arrives \
             as key events and would still work, so every other test here would pass while \
             the launcher was unusable for anyone composing text"
        );
    }

    #[test]
    fn a_preedit_does_not_commit_early() {
        let mut app = focused_app();
        drive(
            &mut app,
            vec![vec![iced_winit::core::Event::InputMethod(
                input_method::Event::Preedit("にほんご".to_owned(), None),
            )]],
        );
        assert_eq!(
            app.query, "",
            "an uncommitted IME pre-edit leaked into the query; the launcher would search \
             for each intermediate composition state"
        );
    }
}
#[cfg(test)]
mod view_tests {
    use super::*;
    use crate::preset::{self, Preset};

    /// A launcher showing results, built through the code `vicinae` uses.
    fn app_showing_results(dir: &std::path::Path, preset_name: &str) -> LauncherApp {
        // Comments matter: the subtitle is what `subtitles` hides, and a
        // fixture without one would make that assertion measure nothing.
        for (file, name, comment) in [
            ("firefox.desktop", "Firefox", "Browse the web"),
            ("files.desktop", "Files", "Access and organize files"),
            ("terminal.desktop", "Terminal", "Run commands"),
        ] {
            std::fs::write(
                dir.join(file),
                format!(
                    "[Desktop Entry]\nType=Application\nName={name}\nComment={comment}\nExec=/bin/true\n"
                ),
            )
            .expect("write entry");
        }
        let mut app = LauncherApp::with_index(AppIndex::builder().dir(dir).build());
        app.apply(AppFlags {
            appearance_preset: preset::resolve(Some(preset_name), None, None),
            ..AppFlags::default()
        });
        // `fi` matches Firefox and Files, so there is a row beneath the first
        // one -- which is the only place a height change is visible.
        let _ = app.update(Message::QueryChanged("fi".to_owned()));
        app
    }

    /// The top of the row whose title is `title`, as the renderer laid it out.
    fn row_top(app: &LauncherApp, title: &str) -> f32 {
        let mut ui = iced_test::simulator(app.view());
        ui.find(title)
            .unwrap_or_else(|error| panic!("no row titled {title:?}: {error:?}"))
            .bounds()
            .y
    }

    #[test]
    fn the_view_renders_and_the_results_are_in_it() {
        // The precondition for everything below. If this fails, the assertions
        // that follow are measuring nothing.
        let dir = tempfile::tempdir().expect("tempdir");
        let app = app_showing_results(dir.path(), "gnome");
        assert!(!app.results.is_empty(), "precondition: there are rows");

        let mut ui = iced_test::simulator(app.view());
        assert!(ui.find("Firefox").is_ok(), "the row should be drawn");
    }

    #[test]
    fn each_notice_state_says_what_it_should() {
        // #13 names "results list, empty state, detail view, form" for this
        // coverage. These are the three the launcher actually has; a detail
        // view and a form do not exist yet, and inventing tests for them would
        // be testing nothing.
        //
        // Each of these is one branch of the same `if` in `view`, and the
        // branches are only distinguishable by the words they draw -- which is
        // exactly what a user distinguishes them by.
        let dir = tempfile::tempdir().expect("tempdir");

        // Empty query: the resting state, and the one most people see most.
        let mut app = app_showing_results(dir.path(), "gnome");
        let _ = app.update(Message::QueryChanged(String::new()));
        let mut ui = iced_test::simulator(app.view());
        assert!(ui.find("Type to search").is_ok());
        assert!(
            ui.find("Files").is_err(),
            "an empty query should draw no rows"
        );

        // A query nothing matches.
        let mut app = app_showing_results(dir.path(), "gnome");
        let _ = app.update(Message::QueryChanged("zzzznotathing".to_owned()));
        let mut ui = iced_test::simulator(app.view());
        assert!(ui.find("No results").is_ok());
        assert!(ui.find("Type to search").is_err());

        // A failed launch, which replaces the list rather than sitting beside
        // it -- worth pinning, because the error hiding the results is a
        // deliberate choice and not obviously the right one.
        let mut app = app_showing_results(dir.path(), "gnome");
        let _ = app.update(Message::Launched(Err("no Exec key".to_owned())));
        let mut ui = iced_test::simulator(app.view());
        assert!(
            ui.find("could not launch: no Exec key").is_ok(),
            "the failure should name itself"
        );
        assert!(
            ui.find("Files").is_err(),
            "the error replaces the list while it is shown"
        );
    }

    #[test]
    fn the_action_panel_draws_over_the_list_rather_than_replacing_it() {
        // The other half of the same claim: `view` stacks the panel on the
        // card instead of swapping it, and the comment in `view` says so. A
        // test that only checked the panel was visible would pass either way.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_showing_results(dir.path(), "gnome");
        let _ = app.update(Message::TogglePanel);
        assert!(app.panel.is_some(), "precondition: the panel opened");

        let mut ui = iced_test::simulator(app.view());
        assert!(ui.find("Open").is_ok(), "the panel's first action");
        assert!(
            ui.find("Files").is_ok(),
            "the list should still be underneath"
        );
    }

    #[test]
    fn the_flow_rule_is_actually_drawn() {
        // THE CONTROL #108 COULD NOT CLOSE.
        //
        // Two launchers identical in every respect except `field_rule`, so the
        // difference measured is that branch and nothing else -- comparing the
        // `flow` and `gnome` presets instead would conflate radius, padding and
        // spacing with the rule.
        //
        // The rule is a one-pixel container between the field and the body, so
        // drawing it moves everything below it down by exactly its height.
        let dir = tempfile::tempdir().expect("tempdir");

        let mut without = app_showing_results(dir.path(), "gnome");
        without.field_rule = false;
        let mut with = app_showing_results(dir.path(), "gnome");
        with.field_rule = true;

        let baseline = row_top(&without, "Firefox");
        let ruled = row_top(&with, "Firefox");

        assert!(
            (ruled - baseline - 1.0).abs() < f32::EPSILON,
            "the rule should push the list down by its own 1px height: \
             {baseline} without, {ruled} with"
        );
    }

    #[test]
    fn hiding_subtitles_removes_them_from_the_drawn_tree() {
        // The trait the rendered surrogate caught by eye and no test could.
        //
        // Asserted by asking the tree for the subtitle's text rather than by
        // measuring anything: rows are a fixed height, so hiding the subtitle
        // re-centres the title inside the row instead of shrinking it, and a
        // geometric assertion here would be measuring the centring. Whether
        // the words are on screen is the thing the option is actually about.
        let dir = tempfile::tempdir().expect("tempdir");

        let mut with = app_showing_results(dir.path(), "gnome");
        with.subtitles = true;
        let mut without = app_showing_results(dir.path(), "gnome");
        without.subtitles = false;

        let mut shown = iced_test::simulator(with.view());
        assert!(
            shown.find("Access and organize files").is_ok(),
            "the subtitle should be drawn when subtitles are on"
        );

        let mut hidden = iced_test::simulator(without.view());
        assert!(
            hidden.find("Access and organize files").is_err(),
            "the subtitle should be gone when subtitles are off"
        );
        // And the title is still there, so the row was not simply dropped.
        assert!(hidden.find("Files").is_ok());
    }

    #[test]
    fn the_dense_preset_really_is_denser_on_screen() {
        // Not a token comparison -- `preset::tests` already does that. This is
        // the laid-out result, which is the thing a user sees.
        //
        // The pitch between two rows, in absolute value: the ranking decides
        // which of the two is on top, and that is not what this is about.
        let dir = tempfile::tempdir().expect("tempdir");
        let gnome = app_showing_results(dir.path(), "gnome");
        let rofi = app_showing_results(dir.path(), "rofi");

        assert!(
            gnome.results.len() >= 2 && rofi.results.len() >= 2,
            "precondition: two rows, or there is no pitch to measure"
        );
        let gnome_pitch = (row_top(&gnome, "Files") - row_top(&gnome, "Firefox")).abs();
        let rofi_pitch = (row_top(&rofi, "Files") - row_top(&rofi, "Firefox")).abs();

        assert!(
            rofi_pitch < gnome_pitch,
            "rofi's rows should be closer together: {rofi_pitch} vs {gnome_pitch}"
        );

        // And each pitch is the preset's own row height plus its spacing.
        //
        // The inequality above is not enough on its own: `rofi` also sets
        // `row_spacing` to 0, so a `view` that ignored `row_height` entirely
        // would still lay rofi out 2 px tighter and satisfy it. A control
        // caught exactly that. Pinning both pitches to their presets' numbers
        // means the height has to be read from the preset for either to hold.
        for (app, preset) in [(&gnome, Preset::Gnome), (&rofi, Preset::Rofi)] {
            let g = preset.geometry();
            let expected = f32::from(g.row_height) + f32::from(g.row_spacing);
            let measured = (row_top(app, "Files") - row_top(app, "Firefox")).abs();
            assert!(
                (measured - expected).abs() < 0.01,
                "{} should lay rows out {expected} apart, measured {measured}",
                preset.name()
            );
        }
    }
}

#[cfg(test)]
mod cold_start_tests {
    use super::*;

    #[test]
    fn the_first_frame_is_recorded_once_and_only_once() {
        // The latch is the whole mechanism. Without it the figure would be
        // rewritten at the refresh rate and report the time between the last
        // two frames rather than the time to the first.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = LauncherApp::with_index(AppIndex::builder().dir(dir.path()).build());
        app.started_at = Some(std::time::Instant::now());

        assert!(app.first_frame_at.is_none(), "nothing drawn yet");

        let _ = app.update(Message::FrameDrawn);
        let first = app.first_frame_at.expect("the first frame was recorded");

        for _ in 0..5 {
            let _ = app.update(Message::FrameDrawn);
        }
        assert_eq!(
            app.first_frame_at,
            Some(first),
            "a later frame must not overwrite the first"
        );
    }

    // NOT TESTED, AND RECORDED RATHER THAN FAKED: that `subscription` stops
    // asking for frames once the first has arrived.
    //
    // `window::frames()` yields at the refresh rate, so leaving it subscribed
    // would push a message sixty times a second for the life of the process.
    // The guard in `subscription` is what prevents that, and removing it is a
    // mutation no test here catches: `iced::Subscription` is opaque -- its
    // `Debug` is the bare string "Subscription" -- so a test cannot see which
    // streams a batch contains. Asserting on a predicate extracted from the
    // guard would test the predicate, not the guard, and would pass with the
    // guard deleted.
    //
    // The cost of the miss is wasted work rather than wrong behaviour: the
    // latch below keeps a late frame from overwriting the figure either way.

    #[test]
    fn without_a_start_time_nothing_is_claimed() {
        // A test, or any caller that is not timing, sets no start. The frame
        // is still latched -- that is what stops the subscription -- but no
        // figure is logged, because there would be nothing to measure from.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = LauncherApp::with_index(AppIndex::builder().dir(dir.path()).build());
        assert!(app.started_at.is_none(), "precondition");

        let _ = app.update(Message::FrameDrawn);
        assert!(
            app.first_frame_at.is_some(),
            "the latch still closes, or the subscription would never stop"
        );
    }
}
