//! Whether a screen reader is running, and what that costs here.
//!
//! Part of [`super`]; see that module for the purity rule every function here
//! keeps to.
//!
//! The Rust launcher has no accessibility tree (ADR-0016, #118), so a screen
//! reader sees nothing of it. That is a known absence rather than a fault, so
//! it is a warning, and only when it matters: when a screen reader is on.

use compass_ipc::{DoctorCheck, DoctorStatus};

use super::check;
use crate::doctor::bus::BusProbe;

/// Bus name of the accessibility bus launcher, which owns the switch.
pub const A11Y_BUS_NAME: &str = "org.a11y.Bus";
/// Object path the switch is exported on.
pub const A11Y_OBJECT_PATH: &str = "/org/a11y/bus";
/// Interface holding the screen-reader switch GNOME's settings flip.
pub const A11Y_STATUS_INTERFACE: &str = "org.a11y.Status";

/// Whether a screen reader is enabled, and so whether the missing tree matters.
pub async fn screen_reader<B: BusProbe>(bus: &B) -> DoctorCheck {
    const NAME: &str = "a11y.screen-reader";
    let enabled = bus
        .property(
            A11Y_BUS_NAME,
            A11Y_OBJECT_PATH,
            A11Y_STATUS_INTERFACE,
            "ScreenReaderEnabled",
        )
        .await;
    match enabled {
        Ok(Some(value)) if value == "true" => check(
            NAME,
            DoctorStatus::Warn,
            "a screen reader is enabled, and this launcher exposes no accessibility tree: it \
             cannot read the query, the results or which one is selected (ADR-0016, #118). \
             The C++ engine is accessible through Qt",
        ),
        Ok(Some(_)) => check(
            NAME,
            DoctorStatus::Ok,
            "no screen reader enabled. This launcher has no accessibility tree yet (#118)",
        ),
        Ok(None) => check(
            NAME,
            DoctorStatus::Ok,
            format!(
                "{A11Y_BUS_NAME} does not report a screen-reader switch, so none is running. \
                 This launcher has no accessibility tree yet (#118)"
            ),
        ),
        Err(err) => check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "could not ask {A11Y_BUS_NAME} whether a screen reader is enabled: {err}. If \
                 one is, it cannot read this launcher (#118)"
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::bus::FakeBus;

    fn reader(value: &str) -> FakeBus {
        FakeBus::new().with_property(
            A11Y_BUS_NAME,
            A11Y_OBJECT_PATH,
            A11Y_STATUS_INTERFACE,
            "ScreenReaderEnabled",
            value,
        )
    }

    #[tokio::test]
    async fn an_enabled_screen_reader_is_warned_about_by_name() {
        let result = screen_reader(&reader("true")).await;
        assert_eq!(result.status, DoctorStatus::Warn);
        let detail = result.detail.expect("detail");
        assert!(detail.contains("#118"), "{detail}");
        assert!(detail.contains("cannot read"), "{detail}");
    }

    #[tokio::test]
    async fn a_disabled_screen_reader_is_fine() {
        assert_eq!(
            screen_reader(&reader("false")).await.status,
            DoctorStatus::Ok
        );
    }

    #[tokio::test]
    async fn no_accessibility_bus_means_no_screen_reader() {
        assert_eq!(
            screen_reader(&FakeBus::new()).await.status,
            DoctorStatus::Ok
        );
    }

    #[tokio::test]
    async fn a_failed_query_is_not_reported_as_all_clear() {
        let result = screen_reader(&FakeBus::failing_queries("timeout")).await;
        assert_eq!(result.status, DoctorStatus::Warn);
    }
}
