//! Diagnostic checks facade re-exporting checks from submodules.

pub mod dbus;
pub mod env;

pub use dbus::*;
pub use env::*;

use compass_ipc::{DoctorCheck, DoctorStatus};

pub(in crate::doctor) fn check(name: &str, status: DoctorStatus, detail: impl Into<String>) -> DoctorCheck {
    DoctorCheck {
        name: name.to_string(),
        status,
        detail: Some(detail.into()),
    }
}
