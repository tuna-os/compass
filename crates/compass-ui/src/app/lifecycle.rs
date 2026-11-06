//! The launcher window's lifecycle: opening it, hiding or exiting on
//! dismissal, obeying the engine's show, hide and toggle, the backdrop that
//! catches a click outside the card, and fitting the window to its content
//! and its monitor.
//!
//! Each command the engine sends blocks it until the UI answers, so every
//! path through [`LauncherApp::obey`] answers exactly once, and a hide is
//! answered only when the window is really gone.

use super::{
    GEOMETRY, LauncherApp, Message, Page, Task, UiCommand, UiOutcome, design, focus_search,
    onboarding, window,
};

/// Set when the app asks its event loop to end.
pub(super) static EXIT_REQUESTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// What a dismissal does. See [`LauncherApp::on_dismiss`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dismissal {
    /// Close the window and wait to be summoned again.
    Hide,
    /// End the process, because nothing could summon it back.
    Exit,
}

impl LauncherApp {
    /// The window's own events: its first frame, opening and closing, the
    /// engine going away and a click on the backdrop.
    pub(super) fn lifecycle_message(&mut self, message: Message) -> Task<Message> {
        match message {
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
            // The elapsed time is from `AppFlags::started_at`, which `compass`
            // takes on entry -- so it excludes dynamic linking, and a wgpu
            // binary's is not free either. The VM tier measures from spawn to
            // this line and catches both.
            //
            // Recorded, not gated: ADR-0010. A threshold comes from the
            // numbers, once there are some.
            Message::FrameDrawn => {
                // SUMMON, RECORDED ON THE SAME TERMS AS COLD START BELOW: from
                // the engine's `Show` reaching this process to the new window's
                // first `RedrawRequested`. Only once that window has opened --
                // a frame from a window still closing is not the summoned one.
                // Logged as `summon_draw_ms`, a floor for the same reason
                // `first_draw_ms` is: painting what was requested is not in it.
                if self.window.is_some()
                    && self.pending_window.is_none()
                    && !self.closing
                    && let Some(summoned) = self.summoned_at.take()
                {
                    let drawn = summoned.elapsed();
                    self.last_summon_draw = Some(drawn);
                    tracing::info!(
                        target: "compass_ui::startup",
                        summon_draw_ms = drawn.as_millis() as u64,
                        "summoned window's first frame requested (a floor on summon latency)"
                    );
                }
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
            Message::CloseWindow(window_id) => {
                // `CLOSE_WINDOW_SHORTCUT` (`ctrl+q`) on a window row.
                let _bounded = format!("CloseWindow({window_id})");
                self.conceal()
            }
            Message::EngineDisconnected => {
                if self.exit_on_engine_disconnect {
                    exit()
                } else {
                    Task::none()
                }
            }
            Message::Opened(id) if self.hud.owns(id) => Task::none(),
            Message::Closed(id) if self.hud.closed(id) => Task::none(),
            Message::Opened(id) if self.backdrop == Some(id) => Task::none(),
            Message::Closed(id) if self.backdrop == Some(id) => {
                self.backdrop = None;
                self.backdrop_size = None;
                Task::none()
            }
            Message::BackdropPressed => {
                if self.is_visible() && !self.closing && !self.choosing_files {
                    self.conceal()
                } else {
                    Task::none()
                }
            }
            Message::Opened(id) => {
                if self.window.is_some_and(|current| current != id)
                    || self.pending_window.is_some_and(|pending| pending != id)
                {
                    return window::close(id);
                }
                self.pending_window = None;
                self.window = Some(id);
                if std::mem::take(&mut self.pending_hide) {
                    return self.conceal();
                }
                // Answers only a `Show` that asked for it. The window opened at
                // boot answers nothing -- see `awaiting`.
                self.answer(UiOutcome::Shown);
                // Covers boot and every summon: `conceal` closes the window, so
                // a summon opens a new one whose field starts unfocused.
                let search = self.search_task();
                if let Some(failure) = self.error_for_summon.take() {
                    self.error = Some(failure);
                }
                Task::batch([
                    focus_search(),
                    search,
                    self.refresh_shortcuts_task(),
                    self.refresh_scripts_task(),
                    self.refresh_rhai_scripts_task(),
                    self.refresh_subtitles_task(),
                    self.refresh_update_task(),
                    self.catalog_task(),
                    self.exchange_rates_task(),
                    self.window_capabilities_task(),
                    self.apply_dmenu_size(),
                ])
            }
            Message::Closed(id) => {
                // Only clear the state if *this* window is the one that went;
                // a stale close for a window already replaced would otherwise
                // leave the launcher believing it is hidden while it is not.
                if self.window == Some(id) {
                    self.cancel_search();
                    self.window = None;
                    self.resized_to = None;
                    // The next window is a new surface, with no blur yet.
                    self.material_asked = None;
                    self.closing = false;
                    self.measuring = None;
                    self.window_size = None;
                    if std::mem::take(&mut self.reopen_after_close) {
                        return self.open_window();
                    }
                    // The honest moment to say "hidden": the window is gone.
                    // Answers only a command that asked -- a window the user
                    // closed answers nothing. See `awaiting`.
                    self.answer(UiOutcome::Hidden);
                    // The compositor may have closed it (its output went
                    // away), and the backdrop must not stay to catch clicks.
                    return self.close_backdrop();
                }
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// Whether a window is currently on screen.
    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.window.is_some()
    }

    /// Answers the engine, if it is waiting for one.
    ///
    /// Does nothing when no command is outstanding. See [`Self::awaiting`] for
    /// why that matters more than it looks.
    pub(super) fn answer(&mut self, outcome: UiOutcome) {
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
    pub(super) fn conceal(&mut self) -> Task<Message> {
        // Dismissing the first-run flow, by Escape or the hotkey, counts as
        // having seen it: it is not shown again at every login. The engine
        // waits for the flow to ask for the launcher hotkey, so closing it
        // asks when nobody did.
        let shortcut = match &self.page {
            Page::Onboarding(page) => {
                onboarding::record_completed(&page.state_path);
                self.ask_for_hotkey_on_finish(page.asks_on_finish())
            }
            _ => Task::none(),
        };
        self.cancel_search();
        self.panel = None;
        // Leaving Set Theme without choosing puts the theme back, as
        // `beforePop` does.
        if matches!(self.page, Page::Themes(_)) {
            let _ = self.update(Message::ThemeCancel);
        }
        self.parked_fonts = None;
        self.parked_store = None;
        self.provider_scope = None;
        self.open_with_return = None;
        let dismissed = self.cancel_dmenu();
        let closing = Task::batch([dismissed, self.close_extension_view()]);
        // A summon starts at the root, whatever view was open when it hid.
        self.page = Page::Root;
        let cleared = if self.pop_to_root_on_close && !self.query.is_empty() {
            self.update(Message::QueryChanged(String::new()))
        } else {
            Task::none()
        };
        let hidden = self.hide_window();
        Task::batch([closing, cleared, hidden, shortcut])
    }

    /// Hides or exits after [`Self::conceal`] has reset the view.
    pub(super) fn hide_window(&mut self) -> Task<Message> {
        self.reopen_after_close = false;
        self.window_focused = false;
        if self.on_dismiss() == Dismissal::Exit {
            return exit();
        }
        if self.pending_window.is_some() {
            self.pending_hide = true;
            return Task::none();
        }
        if self.link.is_none() {
            // Unreachable: `on_dismiss` returns `Hide` only when there is a
            // link. Written as a return rather than an unwrap so a future
            // change to `on_dismiss` degrades into exiting rather than
            // panicking in the middle of a keystroke.
            return exit();
        }
        match self.window {
            // The answer waits for `Message::Closed`, which arrives when the
            // window is actually gone.
            //
            // REPORTING HERE WOULD BE OPTIMISTIC, AND IT WAS. `window::close`
            // returns a Task; answering before it runs tells the engine
            // "hidden" while the window is still on screen. A VM run caught it:
            // `compass toggle` reported success and the screenshot taken
            // straight afterwards still had the launcher in it.
            //
            // `self.window` is deliberately NOT cleared yet. Until the close
            // lands the window really is still visible, and `is_visible` should
            // say so -- the engine cannot send another command in the meantime
            // because it is blocked reading this one's reply.
            Some(id) => {
                self.closing = true;
                Task::batch([window::close(id), self.close_backdrop()])
            }
            // Nothing to close, so nothing to wait for. Still answers, because
            // the engine is blocked until it hears something.
            None => {
                self.answer(UiOutcome::Hidden);
                self.close_backdrop()
            }
        }
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
            crate::surface::opened_events(),
        ];
        // Only while the first frame is still owed. Once it has been reported
        // this stream is dropped, so the per-frame message stops entirely
        // rather than being produced and discarded at the refresh rate.
        if self.first_frame_at.is_none() || self.summoned_at.is_some() {
            streams.push(window::frames().map(|_| Message::FrameDrawn));
        }
        if let Some(link) = &self.link {
            streams.push(
                link.subscription()
                    .map(|command| command.map_or(Message::EngineDisconnected, Message::Command)),
            );
        }
        if let Some(link) = &self.view.appearance_link {
            streams.push(link.subscription().map(Message::AppearanceChanged));
        }
        if let Some(link) = &self.view.typography_link {
            streams.push(link.subscription().map(Message::TypographyChanged));
        }
        if let Some(link) = &self.config_link {
            streams.push(link.subscription().map(Message::ConfigReloaded));
        }
        // Each second while the clock shows; `clock_tick` redraws it only
        // when its interval comes round.
        if self.clock.is_some() && matches!(self.page, Page::Root) {
            streams.push(
                iced::time::every(std::time::Duration::from_secs(1)).map(|_| Message::ClockTick),
            );
        }
        streams.extend(self.hud_subscription());
        if crate::accessibility::installed() {
            streams.push(crate::accessibility::subscription());
        }
        iced::Subscription::batch(streams)
    }

    /// Acts on a command from the engine and reports what happened.
    ///
    /// `Show` on an already-visible window reports `Shown` without opening a
    /// second one: the outcome names the state the window ended in, so
    /// "already there" and "just opened" are the same answer.
    pub(super) fn obey(&mut self, command: UiCommand) -> Task<Message> {
        // From here until the outcome is sent, the engine is blocked reading
        // one reply. Set before any branch so every path answers exactly once.
        self.awaiting = true;

        if command == UiCommand::Describe {
            let open = (self.is_visible() && !self.closing)
                || (self.pending_window.is_some() && !self.pending_hide);
            self.answer(if open {
                UiOutcome::Shown
            } else {
                UiOutcome::Hidden
            });
            return Task::none();
        }

        if let UiCommand::Failure { title, message } = &command {
            let task = self.command_failed(title, message);
            let open = self.is_visible() && !self.closing;
            self.answer(if open {
                UiOutcome::Shown
            } else {
                UiOutcome::Hidden
            });
            return task;
        }

        if let UiCommand::Hud { text, icon } = &command {
            let hud = crate::hud::Hud {
                text: text.clone(),
                icon: icon.clone(),
            };
            let (shown, task) = self.put_up_hud_for_engine(hud);
            // The outcome names the launcher's state, which a HUD leaves alone.
            let open = self.is_visible() && !self.closing;
            self.answer(if shown && open {
                UiOutcome::Shown
            } else if shown {
                UiOutcome::Hidden
            } else {
                UiOutcome::Failed("this session has no HUD".to_owned())
            });
            return task;
        }

        let opened = match &command {
            UiCommand::Dmenu(token) => self.start_dmenu(*token),
            UiCommand::Launch(token) => self.start_launch(*token),
            UiCommand::Deeplink(url) => self.open_deeplink(url),
            _ => Task::none(),
        };
        let shown = self.obey_visibility(&command);
        Task::batch([opened, shown])
    }

    /// The visibility half of [`Self::obey`]: every command but `Hide`
    /// shows the window, `Toggle` depending on where it is.
    pub(super) fn obey_visibility(&mut self, command: &UiCommand) -> Task<Message> {
        let show = match command {
            UiCommand::Show
            | UiCommand::Dmenu(_)
            | UiCommand::Launch(_)
            | UiCommand::Deeplink(_) => true,
            UiCommand::Hide => false,
            // Answered in `obey` without touching the window.
            UiCommand::Describe | UiCommand::Hud { .. } | UiCommand::Failure { .. } => {
                return Task::none();
            }
            UiCommand::Toggle => {
                let open = (self.is_visible() && !self.closing)
                    || (self.pending_window.is_some() && !self.pending_hide)
                    || self.reopen_after_close;
                !open || self.behind_another_window()
            }
        };

        if !show {
            self.summoned_at = None;
            return self.conceal();
        }

        if matches!(command, UiCommand::Show | UiCommand::Toggle)
            && let Some(path) = self.pending_onboarding.take()
        {
            self.open_onboarding(path);
        }

        // A toplevel left behind another window comes back as a new one.
        // Asking the compositor to raise and focus the old one is refused
        // without an activation token (Sway kept the keys with the other
        // window), while a new window is focused like every summon.
        if let (true, Some(id)) = (self.behind_another_window(), self.window) {
            self.summoned_at.get_or_insert_with(std::time::Instant::now);
            self.closing = true;
            self.reopen_after_close = true;
            return window::close(id);
        }

        if self.window.is_none() || self.closing {
            // A summon that has to put a window on screen: the case the
            // resident window (ADR-0015) exists to make fast. A second Show
            // while the first is still opening keeps the earlier start.
            self.summoned_at.get_or_insert_with(std::time::Instant::now);
        }

        if self.pending_window.is_some() {
            self.pending_hide = false;
            return Task::none();
        }
        if self.closing {
            self.reopen_after_close = true;
            return Task::none();
        }

        if let Some(id) = self.window {
            self.answer(UiOutcome::Shown);
            // Both, and they are not the same thing: `gain_focus` raises the
            // WINDOW, `focus_search` focuses the FIELD inside it. A window
            // summoned with only the first is on top and still deaf.
            return Task::batch([window::gain_focus(id), focus_search()]);
        }

        self.open_window()
    }

    pub(super) fn open_window(&mut self) -> Task<Message> {
        let mut settings = self.window_config.clone();
        settings.size = self.wanted_window_size();
        let backdrop = self.open_backdrop();
        let (id, opened) = crate::surface::open(settings);
        self.pending_window = Some(id);
        self.pending_hide = false;
        self.window_size = None;
        self.monitor_asked = None;
        self.focus_seen = false;
        self.measuring = crate::surface::opens_to_measure().then_some(id);
        // Which of two surfaces on one layer is on top is the compositor's
        // call (Sway 1.9 put the one asked for first on top), so the
        // launcher is asked for first, and the
        // backdrop's input region leaves the launcher's rectangle out
        // (`cut_backdrop`) so a click there reaches the launcher either way.
        opened.chain(backdrop)
    }

    /// Leaves the launcher's rectangle out of the backdrop's input region,
    /// once both have their sizes and the launcher is not still measuring.
    pub(super) fn cut_backdrop(&self) -> Task<Message> {
        match (self.backdrop, self.backdrop_size, self.window_size) {
            (Some(backdrop), Some(room), Some(launcher)) if self.measuring.is_none() => {
                crate::surface::cut_backdrop(backdrop, room, launcher)
            }
            _ => Task::none(),
        }
    }

    /// Puts the backdrop under a layer-shell launcher that hides on focus
    /// loss. A layer surface holding the keyboard exclusively never loses
    /// the focus, and a click on another window went through to it and left
    /// the launcher up (TIL-03); the backdrop takes that click instead. A
    /// toplevel loses the focus like any window and needs none.
    pub(super) fn open_backdrop(&mut self) -> Task<Message> {
        if !self.close_on_focus_loss || self.backdrop.is_some() {
            return Task::none();
        }
        let id = window::Id::unique();
        let task = crate::surface::open_backdrop(id);
        if task.units() > 0 {
            self.backdrop = Some(id);
        }
        task
    }

    /// Takes the backdrop down with the launcher.
    pub(super) fn close_backdrop(&mut self) -> Task<Message> {
        self.backdrop_size = None;
        self.backdrop.take().map_or_else(Task::none, window::close)
    }

    /// Whether the launcher is open but another window has the focus: a
    /// toplevel left behind one with `close_on_focus_loss` off. `Toggle`
    /// brings it forward rather than hiding what the user cannot see.
    ///
    /// Only once this window has reported its focus at all: a seat with no
    /// keyboard reports none, and the launcher must still toggle closed
    /// there. A layer surface on top with the keyboard is never behind.
    pub(super) fn behind_another_window(&self) -> bool {
        crate::surface::presentation() == crate::surface::Presentation::Toplevel
            && self.is_visible()
            && !self.closing
            && self.focus_seen
            && !self.window_focused
    }

    /// The window size the current view asks for (the launcher's own, or a
    /// dmenu list's), fitted to the room last seen on the output.
    pub(super) fn wanted_window_size(&self) -> iced::Size {
        let pad = 2 * u32::from(design::SHADOW_PADDING);
        let (width, height) = self.dmenu_card_size().unwrap_or((
            u32::from(GEOMETRY.card_width),
            u32::from(GEOMETRY.card_max_height),
        ));
        crate::surface::fit(
            iced::Size::new((width + pad) as f32, (height + pad) as f32),
            self.room,
        )
    }

    /// The launcher window was opened or resized. A layer surface opened to
    /// measure the output is given its fitted size; one being fitted is
    /// drawn, and focused, once it has it.
    pub(super) fn window_sized(
        &mut self,
        id: window::Id,
        size: iced::Size,
        opened: bool,
    ) -> Task<Message> {
        if self.backdrop == Some(id) {
            self.backdrop_size = Some(size);
            return self.cut_backdrop();
        }
        // Under `iced::application` (`crate::run`) the one window is not
        // tracked by id, and its size is still the launcher's.
        let launcher = match (self.window, self.pending_window) {
            (None, None) => !self.hud.owns(id),
            (window, pending) => window == Some(id) || pending == Some(id),
        };
        if !launcher {
            return Task::none();
        }
        self.window_size = Some(size);
        if self.measuring != Some(id) {
            return self.cut_backdrop();
        }
        if opened {
            self.room = Some(size);
            let fitted = self.wanted_window_size();
            self.resized_to = self.dmenu_card_size();
            if fitted != size {
                return crate::surface::fit_opened(id, fitted);
            }
        }
        self.measuring = None;
        // The field was not in the tree while the window measured.
        Task::batch([focus_search(), self.cut_backdrop()])
    }

    /// The monitor a toplevel is on: its size is the room, and a window
    /// larger than that is shrunk to fit.
    pub(super) fn monitor_sized(
        &mut self,
        id: window::Id,
        monitor: Option<iced::Size>,
    ) -> Task<Message> {
        let Some(monitor) = monitor else {
            return Task::none();
        };
        if self.window != Some(id) {
            return Task::none();
        }
        self.room = Some(monitor);
        let fitted = self.wanted_window_size();
        if self.window_size == Some(fitted) {
            return Task::none();
        }
        crate::surface::resize(id, fitted)
    }

    /// Asks for blur behind the card as it was just laid out, or for none
    /// when the card is opaque; nothing when that is what was last asked.
    pub(super) fn card_measured(&mut self, card: iced::Size) -> Task<Message> {
        let Some(id) = self.window else {
            return Task::none();
        };
        if crate::surface::presentation() != crate::surface::Presentation::Toplevel {
            return Task::none();
        }
        let wanted = crate::material::region(
            self.tint,
            card,
            design::SHADOW_PADDING,
            self.geometry.card_radius,
        );
        if wanted == self.material_asked {
            return Task::none();
        }
        self.material_asked = wanted;
        crate::material::apply(id, wanted)
    }
}

/// Ends the event loop, noting that the app asked for it. `iced_winit` returns
/// `Ok` whether the loop ended on request or because the compositor
/// connection failed, and only this tells the two apart ([`exit_requested`]).
pub(super) fn exit() -> Task<Message> {
    EXIT_REQUESTED.store(true, std::sync::atomic::Ordering::Relaxed);
    iced::exit()
}

/// Whether the app asked its event loop to end, rather than the loop ending
/// under it.
pub(crate) fn exit_requested() -> bool {
    EXIT_REQUESTED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Lifts keyboard and window events out of the runtime's event stream.
///
/// A free function rather than a closure because [`iced::event::listen_with`]
/// takes a plain `fn` pointer. See [`LauncherApp::subscription`] for why the
/// stream is read at this level rather than through `iced::keyboard::listen`.
pub(super) fn keyboard_events(
    event: iced::Event,
    _status: iced::event::Status,
    window: window::Id,
) -> Option<Message> {
    match event {
        iced::Event::Keyboard(event) => Some(Message::Keyboard(event)),
        iced::Event::Window(window::Event::Focused) => Some(Message::WindowFocusChanged(true)),
        iced::Event::Window(window::Event::Unfocused) => Some(Message::WindowFocusChanged(false)),
        iced::Event::Window(window::Event::Opened { size, .. }) => Some(Message::WindowSized {
            id: window,
            size,
            opened: true,
        }),
        iced::Event::Window(window::Event::Resized(size)) => Some(Message::WindowSized {
            id: window,
            size,
            opened: false,
        }),
        _ => None,
    }
}
