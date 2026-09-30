//! The clipboard on a wlroots compositor, over data-control.
//!
//! `ext-data-control-v1` (the standardised protocol) is preferred and
//! `zwlr_data_control_manager_v1` (its wlroots predecessor, all Sway before
//! 1.11) is the fallback. Both let a client that does not have focus watch
//! and set the selection, which is what clipboard history needs and what
//! Mutter will not offer (`PLAN.md` §3.1).
//!
//! # Crate first, and the gap around it
//!
//! Setting, reading and clearing go through `wl-clipboard-rs`, which speaks
//! both protocols and is what `wl-copy`/`wl-paste` are built on. It has no way
//! to **watch** the selection — each call opens a connection, does one thing
//! and closes it — so [`watch`] is the one hand-written part: a device that
//! stays bound and reports each new selection, read with the offer filter the
//! C++ `data-control-server` used ([`crate::data_control`]). `CRATE-AUDIT.md`
//! records why the one crate that does watch (`wayland-clipboard-listener`)
//! was not used.

use std::collections::HashMap;
use std::io::Read;
use std::os::fd::AsFd;
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use tokio::sync::mpsc;
use wayland_client::backend::ObjectId;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, event_created_child};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1::{self, ExtDataControlDeviceV1},
    ext_data_control_manager_v1::ExtDataControlManagerV1,
    ext_data_control_offer_v1::{self, ExtDataControlOfferV1},
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::{self, ZwlrDataControlDeviceV1},
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_offer_v1::{self, ZwlrDataControlOfferV1},
};
use wl_clipboard_rs::copy::{self, MimeSource};
use wl_clipboard_rs::paste;

use crate::data_control::{
    self, CONCEALED_MIME_TYPE, Offer, PASSWORD_HINT_MIME_TYPE, PREFERRED_IMAGE_TYPES,
};

/// How long one MIME type's bytes may take to arrive.
///
/// The source application writes them into a pipe, and an application that
/// has hung would otherwise stall every later selection behind it.
pub const RECEIVE_TIMEOUT: Duration = Duration::from_secs(5);

/// Why a clipboard operation failed.
#[derive(Debug, thiserror::Error)]
pub enum ClipboardError {
    /// No compositor to connect to.
    #[error("no Wayland compositor: {0}")]
    Connect(#[from] wayland_client::ConnectError),
    /// Neither data-control protocol is advertised.
    #[error(
        "the compositor advertises neither ext_data_control_manager_v1 nor \
         zwlr_data_control_manager_v1"
    )]
    Unsupported,
    /// There is no seat, so there is no selection.
    #[error("the compositor has no seat")]
    NoSeat,
    /// The connection failed.
    #[error("Wayland connection: {0}")]
    Connection(String),
    /// `wl-clipboard-rs` refused a copy.
    #[error("setting the clipboard: {0}")]
    Copy(#[from] copy::Error),
    /// `wl-clipboard-rs` refused a read.
    #[error("reading the clipboard: {0}")]
    Paste(#[from] paste::Error),
    /// The bytes did not arrive.
    #[error("reading the clipboard: {0}")]
    Io(#[from] std::io::Error),
}

/// One new selection, as kept by the offer filter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectionChange {
    /// The kept MIME types and their bytes, in lexicographic order.
    pub offers: Vec<Offer>,
}

impl SelectionChange {
    /// Whether the source marked the selection as a secret — a password
    /// manager's hint, or this project's own concealed marker.
    #[must_use]
    pub fn concealed(&self) -> bool {
        self.offers.iter().any(|offer| {
            offer.mime_type == PASSWORD_HINT_MIME_TYPE || offer.mime_type == CONCEALED_MIME_TYPE
        })
    }

    /// The one offer a single-entry history should record.
    ///
    /// The Shell extension hands the engine exactly one `(bytes, type)`, and
    /// the history store takes exactly one, so a multi-type selection is
    /// narrowed here: an image first (the filter has already kept only the
    /// best encoding), then a file list, then UTF-8 text, plain text, HTML,
    /// and otherwise the first kept type that has bytes.
    #[must_use]
    pub fn preferred(&self) -> Option<&Offer> {
        let by_type = |wanted: &str| {
            self.offers
                .iter()
                .find(|offer| offer.mime_type == wanted && !offer.data.is_empty())
        };
        PREFERRED_IMAGE_TYPES
            .iter()
            .find_map(|mime| by_type(mime))
            .or_else(|| by_type("text/uri-list"))
            .or_else(|| by_type("text/plain;charset=utf-8"))
            .or_else(|| by_type("text/plain"))
            .or_else(|| by_type("text/html"))
            .or_else(|| {
                self.offers.iter().find(|offer| {
                    !data_control::is_flag_mime(&offer.mime_type) && !offer.data.is_empty()
                })
            })
    }
}

/// A running selection watcher. Dropping it does not stop the thread; it
/// ends when the compositor connection does.
#[derive(Debug)]
pub struct Watcher {
    protocol: &'static str,
}

impl Watcher {
    /// The data-control protocol in use, by interface name.
    #[must_use]
    pub const fn protocol(&self) -> &'static str {
        self.protocol
    }
}

/// Watches the regular selection of the compositor in `WAYLAND_DISPLAY`.
///
/// Each new selection is read and sent on `changes`. The selection that is
/// current when watching starts is sent too, as data-control announces it on
/// binding. An empty selection (the source exited, or someone cleared it) is
/// not sent: there is nothing to record.
///
/// # Errors
///
/// [`ClipboardError::Unsupported`] when neither protocol is advertised,
/// [`ClipboardError::NoSeat`] without a seat.
pub fn watch(changes: mpsc::UnboundedSender<SelectionChange>) -> Result<Watcher, ClipboardError> {
    watch_on(Connection::connect_to_env()?, changes)
}

/// [`watch`], over an existing connection.
///
/// # Errors
///
/// As [`watch`].
pub fn watch_on(
    connection: Connection,
    changes: mpsc::UnboundedSender<SelectionChange>,
) -> Result<Watcher, ClipboardError> {
    let (globals, mut queue) = registry_queue_init::<Watch>(&connection)
        .map_err(|err| ClipboardError::Connection(err.to_string()))?;
    let qh = queue.handle();

    let seat = globals
        .bind::<wl_seat::WlSeat, _, _>(&qh, 1..=7, ())
        .map_err(|_| ClipboardError::NoSeat)?;
    let protocol =
        if let Ok(manager) = globals.bind::<ExtDataControlManagerV1, _, _>(&qh, 1..=1, ()) {
            manager.get_data_device(&seat, &qh, ());
            "ext_data_control_manager_v1"
        } else if let Ok(manager) = globals.bind::<ZwlrDataControlManagerV1, _, _>(&qh, 1..=2, ()) {
            manager.get_data_device(&seat, &qh, ());
            "zwlr_data_control_manager_v1"
        } else {
            return Err(ClipboardError::Unsupported);
        };

    let mut state = Watch {
        offered: HashMap::new(),
        changes,
    };
    queue
        .roundtrip(&mut state)
        .map_err(|err| ClipboardError::Connection(err.to_string()))?;

    std::thread::Builder::new()
        .name("compass-data-control".to_owned())
        .spawn(move || {
            loop {
                if let Err(err) = queue.blocking_dispatch(&mut state) {
                    tracing::warn!(error = %err, "lost the compositor's selection");
                    break;
                }
                if state.changes.is_closed() {
                    break;
                }
            }
        })
        .map_err(ClipboardError::Io)?;

    Ok(Watcher { protocol })
}

enum AnyOffer {
    Ext(ExtDataControlOfferV1),
    Wlr(ZwlrDataControlOfferV1),
}

impl AnyOffer {
    fn id(&self) -> ObjectId {
        match self {
            Self::Ext(offer) => offer.id(),
            Self::Wlr(offer) => offer.id(),
        }
    }

    fn receive(&self, mime_type: &str, fd: std::os::fd::BorrowedFd<'_>) {
        match self {
            Self::Ext(offer) => offer.receive(mime_type.to_owned(), fd),
            Self::Wlr(offer) => offer.receive(mime_type.to_owned(), fd),
        }
    }

    fn destroy(&self) {
        match self {
            Self::Ext(offer) => offer.destroy(),
            Self::Wlr(offer) => offer.destroy(),
        }
    }
}

struct Watch {
    /// MIME types announced per offer, before its `selection` arrives.
    offered: HashMap<ObjectId, Vec<String>>,
    changes: mpsc::UnboundedSender<SelectionChange>,
}

impl Watch {
    fn selection(&mut self, offer: Option<AnyOffer>, connection: &Connection) {
        let Some(offer) = offer else {
            return;
        };
        let mimes = self.offered.remove(&offer.id()).unwrap_or_default();
        let kept = data_control::filter_mimes(&mimes);
        let selection = data_control::build_selection(&kept, |mime| {
            receive(&offer, mime, connection).unwrap_or_else(|err| {
                tracing::debug!(mime, error = %err, "a clipboard type could not be read");
                Vec::new()
            })
        });
        offer.destroy();
        if selection.offers.is_empty() {
            return;
        }
        let _ = self.changes.send(SelectionChange {
            offers: selection.offers,
        });
    }

    fn discard(&mut self, offer: Option<AnyOffer>) {
        if let Some(offer) = offer {
            self.offered.remove(&offer.id());
            offer.destroy();
        }
    }
}

/// Reads one type's bytes. The source writes into a pipe directly, so no
/// event dispatch is needed while waiting; the read happens on a helper
/// thread so an application that never closes its end costs
/// [`RECEIVE_TIMEOUT`], not the watcher.
fn receive(offer: &AnyOffer, mime: &str, connection: &Connection) -> std::io::Result<Vec<u8>> {
    let (mut reader, writer) = std::io::pipe()?;
    offer.receive(mime, writer.as_fd());
    drop(writer);
    connection
        .flush()
        .map_err(|err| std::io::Error::other(err.to_string()))?;

    let (tx, rx) = std_mpsc::channel();
    std::thread::Builder::new()
        .name("compass-data-control-read".to_owned())
        .spawn(move || {
            let mut data = Vec::new();
            let _ = tx.send(reader.read_to_end(&mut data).map(|_| data));
        })?;
    rx.recv_timeout(RECEIVE_TIMEOUT)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "source did not answer"))?
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Watch {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Watch {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtDataControlManagerV1, ()> for Watch {
    fn event(
        _: &mut Self,
        _: &ExtDataControlManagerV1,
        _: <ExtDataControlManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrDataControlManagerV1, ()> for Watch {
    fn event(
        _: &mut Self,
        _: &ZwlrDataControlManagerV1,
        _: <ZwlrDataControlManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtDataControlDeviceV1, ()> for Watch {
    fn event(
        state: &mut Self,
        _: &ExtDataControlDeviceV1,
        event: ext_data_control_device_v1::Event,
        _: &(),
        connection: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_device_v1::Event::DataOffer { id } => {
                state.offered.insert(id.id(), Vec::new());
            }
            ext_data_control_device_v1::Event::Selection { id } => {
                state.selection(id.map(AnyOffer::Ext), connection);
            }
            ext_data_control_device_v1::Event::PrimarySelection { id } => {
                state.discard(id.map(AnyOffer::Ext));
            }
            ext_data_control_device_v1::Event::Finished => {
                tracing::info!("the compositor withdrew the data-control device");
            }
            _ => {}
        }
    }

    event_created_child!(Watch, ExtDataControlDeviceV1, [
        ext_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ExtDataControlOfferV1, ()),
    ]);
}

impl Dispatch<ZwlrDataControlDeviceV1, ()> for Watch {
    fn event(
        state: &mut Self,
        _: &ZwlrDataControlDeviceV1,
        event: zwlr_data_control_device_v1::Event,
        _: &(),
        connection: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_device_v1::Event::DataOffer { id } => {
                state.offered.insert(id.id(), Vec::new());
            }
            zwlr_data_control_device_v1::Event::Selection { id } => {
                state.selection(id.map(AnyOffer::Wlr), connection);
            }
            zwlr_data_control_device_v1::Event::PrimarySelection { id } => {
                state.discard(id.map(AnyOffer::Wlr));
            }
            zwlr_data_control_device_v1::Event::Finished => {
                tracing::info!("the compositor withdrew the data-control device");
            }
            _ => {}
        }
    }

    event_created_child!(Watch, ZwlrDataControlDeviceV1, [
        zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ZwlrDataControlOfferV1, ()),
    ]);
}

impl Dispatch<ExtDataControlOfferV1, ()> for Watch {
    fn event(
        state: &mut Self,
        offer: &ExtDataControlOfferV1,
        event: ext_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_data_control_offer_v1::Event::Offer { mime_type } = event {
            state.offered.entry(offer.id()).or_default().push(mime_type);
        }
    }
}

impl Dispatch<ZwlrDataControlOfferV1, ()> for Watch {
    fn event(
        state: &mut Self,
        offer: &ZwlrDataControlOfferV1,
        event: zwlr_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = event {
            state.offered.entry(offer.id()).or_default().push(mime_type);
        }
    }
}

/// How long [`set`] waits for the compositor to show the new selection.
pub const SET_SETTLE_TIMEOUT: Duration = Duration::from_millis(500);

/// Puts `offers` on the regular selection, served from a background thread
/// until something else takes the selection.
///
/// `wl-clipboard-rs` returns from a background copy once the request is
/// *queued*, not once the compositor has it, so a read straight afterwards
/// can still see the old selection. Headless Sway showed that in about one
/// run in ten. This waits, up to [`SET_SETTLE_TIMEOUT`], until the selection
/// offers every type that was set; past that it returns anyway, since a
/// client that took the selection in the meantime is not an error.
///
/// # Errors
///
/// [`ClipboardError::Copy`] when there is no data-control, no seat, or the
/// offers are empty.
pub fn set(offers: Vec<Offer>) -> Result<(), ClipboardError> {
    let wanted: Vec<String> = offers.iter().map(|offer| offer.mime_type.clone()).collect();
    let sources = offers
        .into_iter()
        .map(|offer| MimeSource {
            source: copy::Source::Bytes(offer.data.into_boxed_slice()),
            mime_type: copy::MimeType::Specific(offer.mime_type),
        })
        .collect();
    copy::Options::new().copy_multi(sources)?;

    let deadline = std::time::Instant::now() + SET_SETTLE_TIMEOUT;
    while std::time::Instant::now() < deadline {
        let offered = mime_types().unwrap_or_default();
        if wanted.iter().all(|mime| offered.contains(mime)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

/// Clears the regular selection.
///
/// # Errors
///
/// [`ClipboardError::Copy`] when there is no data-control or no seat.
pub fn clear() -> Result<(), ClipboardError> {
    copy::clear(copy::ClipboardType::Regular, copy::Seat::All)?;
    Ok(())
}

/// The MIME types on the regular selection; empty when there is none.
///
/// # Errors
///
/// [`ClipboardError::Paste`] when there is no data-control or no seat.
pub fn mime_types() -> Result<Vec<String>, ClipboardError> {
    match paste::get_mime_types(paste::ClipboardType::Regular, paste::Seat::Unspecified) {
        Ok(types) => Ok(types.into_iter().collect()),
        Err(paste::Error::ClipboardEmpty | paste::Error::NoSeats) => Ok(Vec::new()),
        Err(err) => Err(err.into()),
    }
}

/// The bytes behind `mime_type` on the regular selection, or `None` when the
/// selection is empty or does not offer it.
///
/// `text/plain` is special-cased the way `wl-paste` does it: any of the
/// common plain-text spellings is accepted, UTF-8 first.
///
/// # Errors
///
/// [`ClipboardError::Paste`] when there is no data-control or no seat,
/// [`ClipboardError::Io`] when the bytes do not arrive.
pub fn read(mime_type: &str) -> Result<Option<Vec<u8>>, ClipboardError> {
    let wanted = if mime_type == "text/plain" {
        paste::MimeType::Text
    } else {
        paste::MimeType::Specific(mime_type)
    };
    match paste::get_contents(
        paste::ClipboardType::Regular,
        paste::Seat::Unspecified,
        wanted,
    ) {
        Ok((mut pipe, _)) => {
            let mut data = Vec::new();
            pipe.read_to_end(&mut data)?;
            Ok(Some(data))
        }
        Err(paste::Error::ClipboardEmpty | paste::Error::NoMimeType | paste::Error::NoSeats) => {
            Ok(None)
        }
        Err(err) => Err(err.into()),
    }
}

/// The primary selection's text — what was last selected, in any window —
/// or `None` when nothing is selected, it is not text, or the compositor's
/// data-control is too old to carry the primary selection (version 2 of the
/// wlr protocol added it; the ext protocol always has it).
///
/// # Errors
///
/// [`ClipboardError::Paste`] when there is no data-control,
/// [`ClipboardError::Io`] when the bytes do not arrive.
pub fn read_primary_text() -> Result<Option<String>, ClipboardError> {
    match paste::get_contents(
        paste::ClipboardType::Primary,
        paste::Seat::Unspecified,
        paste::MimeType::Text,
    ) {
        Ok((mut pipe, _)) => {
            let mut data = Vec::new();
            pipe.read_to_end(&mut data)?;
            Ok(Some(String::from_utf8_lossy(&data).into_owned()).filter(|text| !text.is_empty()))
        }
        Err(
            paste::Error::ClipboardEmpty
            | paste::Error::NoMimeType
            | paste::Error::NoSeats
            | paste::Error::PrimarySelectionUnsupported,
        ) => Ok(None),
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(mime: &str, data: &[u8]) -> Offer {
        Offer {
            mime_type: mime.to_owned(),
            data: data.to_vec(),
        }
    }

    #[test]
    fn an_image_is_preferred_over_the_text_beside_it() {
        let change = SelectionChange {
            offers: vec![
                offer("image/png", b"\x89PNG"),
                offer("text/plain;charset=utf-8", b"a.png"),
            ],
        };
        assert_eq!(change.preferred().unwrap().mime_type, "image/png");
    }

    #[test]
    fn utf8_text_is_preferred_over_plain_and_html() {
        let change = SelectionChange {
            offers: vec![
                offer("text/html", b"<b>x</b>"),
                offer("text/plain", b"x"),
                offer("text/plain;charset=utf-8", b"x"),
            ],
        };
        assert_eq!(
            change.preferred().unwrap().mime_type,
            "text/plain;charset=utf-8"
        );
    }

    #[test]
    fn a_type_that_returned_nothing_is_not_preferred() {
        let change = SelectionChange {
            offers: vec![
                offer("text/plain;charset=utf-8", b""),
                offer("text/html", b"<i>y</i>"),
            ],
        };
        assert_eq!(change.preferred().unwrap().mime_type, "text/html");
    }

    #[test]
    fn a_password_hint_or_concealed_marker_conceals_the_selection() {
        let plain = SelectionChange {
            offers: vec![offer("text/plain", b"hunter2")],
        };
        assert!(!plain.concealed());
        for flag in [PASSWORD_HINT_MIME_TYPE, CONCEALED_MIME_TYPE] {
            let flagged = SelectionChange {
                offers: vec![offer(flag, b""), offer("text/plain", b"hunter2")],
            };
            assert!(flagged.concealed(), "{flag}");
        }
    }

    #[test]
    fn a_flag_type_is_never_the_preferred_offer() {
        let change = SelectionChange {
            offers: vec![offer(PASSWORD_HINT_MIME_TYPE, b"")],
        };
        assert_eq!(change.preferred(), None);
    }
}
