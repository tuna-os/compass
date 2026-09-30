//! `compass-wayland` — Wayland integration for the launcher.
//!
//! This crate provides integration points for Wayland protocols that the
//! launcher needs beyond what Iced/winit provides out of the box:
//! - `xdg-activation-v1` for requesting focus with activation tokens
//! - [`keyboard_inhibit`]: `keyboard-shortcuts-inhibit-v1`, the compositor's
//!   shortcuts going to the launcher while it records one
//!
//! and the wlroots track (`PLAN.md` Phase 5, Track B), each chosen from what
//! the compositor advertises ([`compositor`]) and never on GNOME:
//! - [`toplevel`]: window listing, focus and close over
//!   `zwlr_foreign_toplevel_manager_v1`, or listing alone over
//!   `ext_foreign_toplevel_list_v1`
//! - [`clipboard`]: watching and setting the selection over
//!   `ext-data-control-v1` / `zwlr_data_control_manager_v1`
//! - [`hotkey`]: a global hotkey over `xx-hotkey-v1`, and the manual-binding
//!   fallback where there is none
//! - [`virtual_keyboard`]: pressing the paste chord over
//!   `zwp_virtual_keyboard_v1`
//! - [`layer_shell`]: the decision to present the launcher as a layer surface
//!   (the surface itself is `iced_layershell`, in `compass-ui`)
//!
//! The actual window creation is handled by Iced. This crate only adds the
//! protocol integrations.

#![deny(missing_docs)]

pub mod activation;
pub mod clipboard;
pub mod compositor;
pub mod data_control;
pub mod hotkey;
pub mod keyboard_inhibit;
pub mod layer_shell;
pub mod material;
pub mod output;
pub mod toplevel;
pub mod virtual_keyboard;

pub use activation::{ActivationError, ActivationManager, ActivationToken};
pub use compositor::{Capabilities, Family, Globals, Session};
pub use keyboard_inhibit::{InhibitError, InhibitHandle, InhibitState, ShortcutInhibit};
pub use layer_shell::{SurfaceKind, decide_surface, select_surface, should_use_layer_shell};
pub use toplevel::{Toplevel, ToplevelError, Toplevels};
