//! Two per-window registries: who is holding the keyboard, and what is blurred
//! behind what.
//!
//! Ports of `WaylandShortcutInhibitManager`
//! (`src/server/src/services/shortcut-inhibit/`) and
//! `ExtBackgroundEffectV1Manager` (`src/server/src/services/window-material/`),
//! minus the Wayland protocol objects.
//!
//! # What matters is that the bookkeeping is exact
//!
//! Both of these hand out a resource the compositor owns, keyed by window, and
//! both fail in the same two ways if the bookkeeping slips. Ask twice and you
//! leak an inhibitor — on shortcut inhibit that means the keyboard stays
//! grabbed after the window is gone, which locks a person out of their own
//! desktop shortcuts. Ask again with the same arguments and you re-upload a
//! blur region every frame, which is a visible stutter with no error anywhere.
//!
//! Neither of those shows up in a log. They show up as "my Super key stopped
//! working" and "the launcher got slow", so the registry is what ports and
//! what gets the tests.

use std::collections::HashMap;

/// A window, identified the way the C++ identifies one: by pointer identity.
///
/// Modelled as an opaque id so a test can hold two distinct windows without a
/// windowing system.
pub type WindowId = u64;

/// Holds the keyboard for whichever windows asked for it.
#[derive(Debug, Clone, Default)]
pub struct ShortcutInhibitManager {
    /// Whether the compositor offers the protocol at all.
    supported: bool,
    /// The windows currently inhibiting.
    inhibited: Vec<WindowId>,
}

impl ShortcutInhibitManager {
    /// A manager for a compositor that does or does not support the protocol.
    #[must_use]
    pub const fn new(supported: bool) -> Self {
        Self {
            supported,
            inhibited: Vec::new(),
        }
    }

    /// Whether the compositor offers the protocol.
    #[must_use]
    pub const fn is_supported(&self) -> bool {
        self.supported
    }

    /// Note that the protocol appeared or went away.
    ///
    /// The C++ reads `m_manager.isActive()` on every call rather than caching
    /// it, because a Wayland global can be withdrawn while the process runs.
    /// Modelling support as fixed would hide the case the ordering rule below
    /// exists for: a window still holding the keyboard when the protocol has
    /// gone.
    pub fn set_supported(&mut self, supported: bool) {
        self.supported = supported;
    }

    /// Whether `window` is currently inhibiting shortcuts.
    #[must_use]
    pub fn is_inhibiting(&self, window: WindowId) -> bool {
        self.inhibited.contains(&window)
    }

    /// How many windows are inhibiting.
    #[must_use]
    pub fn inhibitor_count(&self) -> usize {
        self.inhibited.len()
    }

    /// Take the keyboard for `window`.
    ///
    /// A window that already has it is told `true` without a second inhibitor
    /// being created, and that check comes *before* the support check — so a
    /// second ask is idempotent even on a compositor that has since stopped
    /// answering. See [`WindowMaterialManager::apply`], which checks the other
    /// way round, for why the asymmetry is right.
    pub fn inhibit(&mut self, window: WindowId) -> bool {
        if self.is_inhibiting(window) {
            return true;
        }
        if !self.supported {
            return false;
        }
        self.inhibited.push(window);
        true
    }

    /// Give the keyboard back.
    ///
    /// Returns whether there was anything to give back. `false` is not a
    /// failure — it is "this window was not holding it" — which is why the
    /// caller can release unconditionally on the way out.
    pub fn release(&mut self, window: WindowId) -> bool {
        match self.inhibited.iter().position(|held| *held == window) {
            Some(index) => {
                self.inhibited.remove(index);
                true
            }
            None => false,
        }
    }

    /// The window's surface was destroyed under us.
    ///
    /// The C++ installs an event filter for exactly this: a window can go away
    /// without anyone calling `release`, and an inhibitor outliving its
    /// surface is how the keyboard stays grabbed with nothing to give it back.
    pub fn surface_destroyed(&mut self, window: WindowId) {
        self.release(window);
    }
}

/// A rectangle, as a blur region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width.
    pub width: i32,
    /// Height.
    pub height: i32,
}

/// What a window's background effect is set to.
///
/// Compared by value, because that comparison is the whole optimisation: the
/// C++ re-applies only when the parameters actually differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MaterialParams {
    /// The blur radius.
    pub radius: i32,
    /// The region blurred.
    pub region: Rect,
}

/// What applying a material led to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    /// A new effect was created for this window.
    Created,
    /// The window already had one and its parameters changed, so it was
    /// re-applied.
    Updated,
    /// The window already had one with these exact parameters; nothing was
    /// sent.
    Unchanged,
    /// This compositor cannot blur.
    Unsupported,
}

impl Applied {
    /// What `apply` returns: whether the window now has the effect.
    #[must_use]
    pub const fn succeeded(self) -> bool {
        !matches!(self, Self::Unsupported)
    }
}

/// Keeps a blur effect per window.
#[derive(Debug, Clone, Default)]
pub struct WindowMaterialManager {
    /// Whether the compositor advertises blur.
    supported: bool,
    /// The parameters each window's effect currently has.
    states: HashMap<WindowId, MaterialParams>,
}

impl WindowMaterialManager {
    /// A manager for a compositor that does or does not support blur.
    #[must_use]
    pub fn new(supported: bool) -> Self {
        Self {
            supported,
            states: HashMap::new(),
        }
    }

    /// Whether the compositor advertises blur.
    ///
    /// The C++ needs both an active manager object *and* the blur capability
    /// bit, because the protocol can be present while blur is not.
    #[must_use]
    pub const fn is_supported(&self) -> bool {
        self.supported
    }

    /// Note that blur appeared or went away.
    pub fn set_supported(&mut self, supported: bool) {
        self.supported = supported;
    }

    /// The parameters `window`'s effect has, if it has one.
    #[must_use]
    pub fn params(&self, window: WindowId) -> Option<MaterialParams> {
        self.states.get(&window).copied()
    }

    /// How many windows have an effect.
    #[must_use]
    pub fn effect_count(&self) -> usize {
        self.states.len()
    }

    /// Give `window` a background effect with `params`.
    /// # The support check comes first here, and second in the inhibitor
    ///
    /// `apply` returns early on `!isSupported()` *before* looking at the
    /// registry, so a compositor that stops advertising blur makes even a
    /// window that already has an effect report failure.
    /// [`ShortcutInhibitManager::inhibit`] is the other way round: it answers
    /// `true` for a window already holding the keyboard whatever the
    /// compositor now says. The asymmetry is in the C++ and it is the right
    /// way round — a stale blur is cosmetic, a stale keyboard grab is a person
    /// locked out of their shortcuts, so the inhibitor must never pretend it
    /// has released one it still holds.
    pub fn apply(&mut self, window: WindowId, params: MaterialParams) -> Applied {
        if !self.supported {
            return Applied::Unsupported;
        }

        if let Some(existing) = self.states.get_mut(&window) {
            if *existing == params {
                return Applied::Unchanged;
            }
            *existing = params;
            return Applied::Updated;
        }

        self.states.insert(window, params);
        Applied::Created
    }

    /// Take the effect away.
    ///
    /// Returns whether there was one, so a caller can clear unconditionally.
    pub fn clear(&mut self, window: WindowId) -> bool {
        self.states.remove(&window).is_some()
    }

    /// The window's surface was destroyed under us.
    pub fn surface_destroyed(&mut self, window: WindowId) {
        self.clear(window);
    }
}
