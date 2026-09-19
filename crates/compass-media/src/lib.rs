//! Media players, over MPRIS.
//!
//! Ports `MprisMediaControl` and `mpris::parseTrackInfo`
//! (`src/server/src/services/media-control/mpris/`). A player is any bus name
//! starting `org.mpris.MediaPlayer2.`, serving
//! `/org/mpris/MediaPlayer2`.
//!
//! # The metadata is the hard part, and it is all corner cases
//!
//! `xesam:artist` is specified as an array of strings and is, in the wild,
//! sometimes a bare string. The C++ handles both and joins with `", "` after
//! dropping empties, then falls back to `xesam:albumArtist` when the result is
//! empty. Every one of those three rules exists because some player does
//! something different, so all three are reproduced and tested.
//!
//! # Identity comes from two places, in order
//!
//! `queryPlayer` starts with the bus name minus its prefix and then, if the
//! root `org.mpris.MediaPlayer2` interface offers a non-empty `Identity`,
//! replaces it. A player that serves no root interface, or an empty
//! `Identity`, keeps the bus-name suffix rather than showing nothing.

use std::collections::HashMap;

use zbus::zvariant::{OwnedValue, Value};

/// Every MPRIS bus name starts with this.
pub const NAME_PREFIX: &str = "org.mpris.MediaPlayer2.";
/// The object every player serves.
pub const PATH: &str = "/org/mpris/MediaPlayer2";
/// The root interface.
pub const ROOT_INTERFACE: &str = "org.mpris.MediaPlayer2";
/// The playback interface.
pub const PLAYER_INTERFACE: &str = "org.mpris.MediaPlayer2.Player";

/// How long to wait for a player.
///
/// The C++ comment says why: "players are third party programs: an
/// unresponsive one must not hang the launcher".
pub const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1000);

/// What a player is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaybackStatus {
    /// Playing.
    Playing,
    /// Paused.
    Paused,
    /// Stopped — and also what an unrecognised value means.
    #[default]
    Stopped,
}

impl PlaybackStatus {
    /// Reads MPRIS's `PlaybackStatus` string.
    ///
    /// Anything other than `Playing` or `Paused` is `Stopped`, which is the
    /// C++'s final `return`.
    #[must_use]
    pub fn parse(status: &str) -> Self {
        match status {
            "Playing" => Self::Playing,
            "Paused" => Self::Paused,
            _ => Self::Stopped,
        }
    }
}

/// One player.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MediaPlayer {
    /// The bus name.
    pub id: String,
    /// What to call it: `Identity`, or the bus name's suffix.
    pub identity: String,
    /// `DesktopEntry`, when the player offers one.
    pub app_id: String,
    /// The track's title.
    pub title: String,
    /// The artists, joined with `", "`.
    pub artist: String,
    /// What it is doing.
    pub status: PlaybackStatus,
    /// Whether `Next` is worth offering.
    pub can_go_next: bool,
    /// Whether `Previous` is worth offering.
    pub can_go_previous: bool,
}

/// Whether a bus name is a player's.
#[must_use]
pub fn is_player_name(service: &str) -> bool {
    service.starts_with(NAME_PREFIX)
}

/// The fallback identity: the bus name with its prefix removed.
#[must_use]
pub fn identity_from_name(service: &str) -> String {
    service
        .strip_prefix(NAME_PREFIX)
        .unwrap_or(service)
        .to_owned()
}

/// Unwraps a variant that holds a variant.
///
/// A property read through `GetAll` arrives as a variant, and a value nested
/// inside it — `Metadata`'s dict, an artist array — arrives as a variant
/// inside that. Matching the outer one alone finds a `Value::Value` and reads
/// nothing, which presents as a track with no title rather than as an error.
fn unwrap_variant(value: Value<'static>) -> Value<'static> {
    match value {
        Value::Value(inner) => unwrap_variant(*inner),
        other => other,
    }
}

/// A string, whatever shape the player put it in.
fn as_string(value: &OwnedValue) -> Option<String> {
    match unwrap_variant(Value::from(value.clone())) {
        Value::Str(text) => Some(text.to_string()),
        _ => None,
    }
}

/// A list of strings, tolerating a single string in place of an array.
///
/// `toStringList` in the C++ does exactly this, and the comment explains the
/// first arm: "string arrays nested in a variant reach us as an undemarshalled
/// argument".
fn as_string_list(value: &OwnedValue) -> Vec<String> {
    match unwrap_variant(Value::from(value.clone())) {
        Value::Str(text) => vec![text.to_string()],
        Value::Array(array) => array
            .iter()
            .filter_map(|item| match unwrap_variant(item.clone()) {
                Value::Str(text) => Some(text.to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The artists of a metadata map, joined as the C++ joins them.
///
/// Empty strings are dropped first, so `["", "Someone"]` is `"Someone"` and
/// not `", Someone"`.
#[must_use]
pub fn join_artists(value: Option<&OwnedValue>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    as_string_list(value)
        .into_iter()
        .filter(|artist| !artist.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Title and artist, out of an MPRIS `Metadata` map.
///
/// `xesam:albumArtist` is the fallback when `xesam:artist` yields nothing.
#[must_use]
pub fn track_info(metadata: &HashMap<String, OwnedValue>) -> (String, String) {
    let title = metadata
        .get("xesam:title")
        .and_then(as_string)
        .unwrap_or_default();

    let mut artist = join_artists(metadata.get("xesam:artist"));
    if artist.is_empty() {
        artist = join_artists(metadata.get("xesam:albumArtist"));
    }

    (title, artist)
}

/// Builds a player from the two property maps.
///
/// `root` is `org.mpris.MediaPlayer2`'s, and may be absent: a player that does
/// not serve it keeps the identity taken from its bus name.
#[must_use]
pub fn player_from_properties(
    service: &str,
    player: &HashMap<String, OwnedValue>,
    root: Option<&HashMap<String, OwnedValue>>,
) -> MediaPlayer {
    let metadata = player
        .get("Metadata")
        .map(|value| match unwrap_variant(Value::from(value.clone())) {
            Value::Dict(dict) => dict
                .iter()
                .filter_map(|(key, value)| {
                    let key = match unwrap_variant(key.clone()) {
                        Value::Str(text) => text.to_string(),
                        _ => return None,
                    };
                    OwnedValue::try_from(value.clone())
                        .ok()
                        .map(|value| (key, value))
                })
                .collect(),
            _ => HashMap::new(),
        })
        .unwrap_or_default();

    let (title, artist) = track_info(&metadata);

    let mut result = MediaPlayer {
        id: service.to_owned(),
        identity: identity_from_name(service),
        app_id: String::new(),
        title,
        artist,
        status: player
            .get("PlaybackStatus")
            .and_then(as_string)
            .map_or(PlaybackStatus::Stopped, |status| {
                PlaybackStatus::parse(&status)
            }),
        can_go_next: player.get("CanGoNext").is_some_and(as_bool),
        can_go_previous: player.get("CanGoPrevious").is_some_and(as_bool),
    };

    if let Some(root) = root {
        if let Some(identity) = root.get("Identity").and_then(as_string)
            && !identity.is_empty()
        {
            result.identity = identity;
        }
        result.app_id = root
            .get("DesktopEntry")
            .and_then(as_string)
            .unwrap_or_default();
    }

    result
}

fn as_bool(value: &OwnedValue) -> bool {
    matches!(
        unwrap_variant(Value::from(value.clone())),
        Value::Bool(true)
    )
}

/// Why a media call failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The bus refused, or the player answered with an error.
    #[error("the player refused: {0}")]
    Bus(#[from] zbus::Error),

    /// The player did not answer within [`CALL_TIMEOUT`].
    ///
    /// Not a theoretical case: a bus name whose owner serves no object never
    /// replies at all, and without this the launcher waits for ever. The C++
    /// passes `CALL_TIMEOUT_MS` to every `QDBusConnection::call` for the same
    /// reason, and its comment says so: "players are third party programs: an
    /// unresponsive one must not hang the launcher".
    #[error("{service} did not answer within {CALL_TIMEOUT:?}")]
    Timeout {
        /// Which player.
        service: String,
    },
}

/// A view of every MPRIS player on a bus.
#[derive(Debug)]
pub struct MediaControl {
    connection: zbus::Connection,
}

impl MediaControl {
    /// Wraps a session-bus connection.
    #[must_use]
    pub fn new(connection: zbus::Connection) -> Self {
        Self { connection }
    }

    /// Every bus name that looks like a player's.
    ///
    /// # Errors
    ///
    /// [`Error::Bus`] if the bus will not list its names.
    pub async fn player_names(&self) -> Result<Vec<String>, Error> {
        let reply = self
            .connection
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "ListNames",
                &(),
            )
            .await?;
        let names: Vec<String> = reply.body().deserialize()?;

        Ok(names
            .into_iter()
            .filter(|name| is_player_name(name))
            .collect())
    }

    async fn properties(
        &self,
        service: &str,
        interface: &str,
    ) -> Result<HashMap<String, OwnedValue>, Error> {
        let body = (interface,);
        let call = self.connection.call_method(
            Some(service),
            PATH,
            Some("org.freedesktop.DBus.Properties"),
            "GetAll",
            &body,
        );
        let reply = tokio::time::timeout(CALL_TIMEOUT, call)
            .await
            .map_err(|_| Error::Timeout {
                service: service.to_owned(),
            })??;
        Ok(reply.body().deserialize()?)
    }

    /// Everything known about one player.
    ///
    /// A player that does not answer the root interface is still returned,
    /// with the identity taken from its bus name — the C++ treats the root
    /// call as optional in exactly that way.
    ///
    /// # Errors
    ///
    /// [`Error::Bus`] if the player does not answer the `Player` interface at
    /// all, which is how an unresponsive or departed player presents.
    pub async fn player(&self, service: &str) -> Result<MediaPlayer, Error> {
        let player = self.properties(service, PLAYER_INTERFACE).await?;
        let root = self.properties(service, ROOT_INTERFACE).await.ok();
        Ok(player_from_properties(service, &player, root.as_ref()))
    }

    /// Every player that answers.
    ///
    /// One that does not is left out rather than failing the call: a launcher
    /// showing three of four players is better than one showing none.
    ///
    /// # Errors
    ///
    /// [`Error::Bus`] if the bus will not list its names.
    pub async fn players(&self) -> Result<Vec<MediaPlayer>, Error> {
        let mut players = Vec::new();
        for name in self.player_names().await? {
            if let Ok(player) = self.player(&name).await {
                players.push(player);
            }
        }
        Ok(players)
    }

    /// Calls a method with no arguments on a player.
    ///
    /// # Errors
    ///
    /// [`Error::Bus`] if the player refuses or is gone.
    pub async fn control(&self, service: &str, method: &str) -> Result<(), Error> {
        let call =
            self.connection
                .call_method(Some(service), PATH, Some(PLAYER_INTERFACE), method, &());
        tokio::time::timeout(CALL_TIMEOUT, call)
            .await
            .map_err(|_| Error::Timeout {
                service: service.to_owned(),
            })??;
        Ok(())
    }

    /// `PlayPause`.
    ///
    /// # Errors
    ///
    /// As [`control`](Self::control).
    pub async fn play_pause(&self, service: &str) -> Result<(), Error> {
        self.control(service, "PlayPause").await
    }

    /// `Next`.
    ///
    /// # Errors
    ///
    /// As [`control`](Self::control).
    pub async fn next(&self, service: &str) -> Result<(), Error> {
        self.control(service, "Next").await
    }

    /// `Previous`.
    ///
    /// # Errors
    ///
    /// As [`control`](Self::control).
    pub async fn previous(&self, service: &str) -> Result<(), Error> {
        self.control(service, "Previous").await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(text: &str) -> OwnedValue {
        OwnedValue::try_from(Value::from(text)).expect("a string value")
    }

    fn list(items: &[&str]) -> OwnedValue {
        let array: Vec<String> = items.iter().map(|s| (*s).to_owned()).collect();
        OwnedValue::try_from(Value::from(array)).expect("a string array")
    }

    #[test]
    fn a_player_is_recognised_by_its_bus_name() {
        assert!(is_player_name("org.mpris.MediaPlayer2.spotify"));
        assert!(!is_player_name("org.gnome.Shell"));
        assert!(
            !is_player_name("org.mpris.MediaPlayer"),
            "the prefix ends with a dot, and a near miss is not a player"
        );
        assert_eq!(
            identity_from_name("org.mpris.MediaPlayer2.vlc.instance42"),
            "vlc.instance42"
        );
    }

    #[test]
    fn the_playback_status_strings_are_mpriss() {
        assert_eq!(PlaybackStatus::parse("Playing"), PlaybackStatus::Playing);
        assert_eq!(PlaybackStatus::parse("Paused"), PlaybackStatus::Paused);
        assert_eq!(PlaybackStatus::parse("Stopped"), PlaybackStatus::Stopped);
        assert_eq!(
            PlaybackStatus::parse("playing"),
            PlaybackStatus::Stopped,
            "MPRIS spells it with a capital; anything else is Stopped"
        );
    }

    #[test]
    fn an_artist_may_be_a_list_or_a_bare_string() {
        // Both are in the wild, and the C++ handles both.
        let mut metadata = HashMap::new();
        metadata.insert("xesam:artist".to_owned(), list(&["A", "B"]));
        assert_eq!(track_info(&metadata).1, "A, B");

        let mut metadata = HashMap::new();
        metadata.insert("xesam:artist".to_owned(), value("Solo"));
        assert_eq!(track_info(&metadata).1, "Solo");
    }

    #[test]
    fn empty_artists_are_dropped_before_joining() {
        // `artists.removeAll(QString{})` -- without it, a list with a blank in
        // it renders as ", Someone".
        let mut metadata = HashMap::new();
        metadata.insert("xesam:artist".to_owned(), list(&["", "Someone", ""]));
        assert_eq!(track_info(&metadata).1, "Someone");
    }

    #[test]
    fn the_album_artist_is_the_fallback() {
        let mut metadata = HashMap::new();
        metadata.insert("xesam:artist".to_owned(), list(&[]));
        metadata.insert("xesam:albumArtist".to_owned(), list(&["Orchestra"]));
        assert_eq!(track_info(&metadata).1, "Orchestra");

        // And only a fallback: a present artist wins.
        metadata.insert("xesam:artist".to_owned(), value("Soloist"));
        assert_eq!(track_info(&metadata).1, "Soloist");
    }

    #[test]
    fn missing_metadata_is_empty_rather_than_absent() {
        let (title, artist) = track_info(&HashMap::new());
        assert_eq!(title, "");
        assert_eq!(artist, "");
    }

    #[test]
    fn the_identity_falls_back_to_the_bus_name() {
        let player = HashMap::new();
        let built = player_from_properties("org.mpris.MediaPlayer2.mpv", &player, None);
        assert_eq!(built.identity, "mpv");
        assert_eq!(built.app_id, "");

        let mut root = HashMap::new();
        root.insert("Identity".to_owned(), value(""));
        let built = player_from_properties("org.mpris.MediaPlayer2.mpv", &player, Some(&root));
        assert_eq!(
            built.identity, "mpv",
            "an empty Identity must not blank the name"
        );

        root.insert("Identity".to_owned(), value("mpv Media Player"));
        root.insert("DesktopEntry".to_owned(), value("mpv"));
        let built = player_from_properties("org.mpris.MediaPlayer2.mpv", &player, Some(&root));
        assert_eq!(built.identity, "mpv Media Player");
        assert_eq!(built.app_id, "mpv");
    }
}
