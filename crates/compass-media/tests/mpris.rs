//! MPRIS against a real bus and two mock players.
//!
//! The unit tests cover the metadata rules, which are where the corner cases
//! are. This covers the part that only a bus can answer: that a player is
//! found by listing names, that `GetAll` on two interfaces decodes into one
//! player, and that a player which does not answer is left out rather than
//! failing the whole list.

mod support;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use compass_media::{MediaControl, PlaybackStatus};
use support::bus;
use zbus::zvariant::{OwnedValue, Value};

struct MockPlayer {
    status: String,
    calls: Arc<Mutex<Vec<String>>>,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl MockPlayer {
    #[zbus(property)]
    fn playback_status(&self) -> String {
        self.status.clone()
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        let mut metadata = HashMap::new();
        metadata.insert(
            "xesam:title".to_owned(),
            OwnedValue::try_from(Value::from("A Song")).expect("a value"),
        );
        metadata.insert(
            "xesam:artist".to_owned(),
            OwnedValue::try_from(Value::from(vec!["First".to_owned(), "Second".to_owned()]))
                .expect("a value"),
        );
        metadata
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        false
    }

    #[zbus(name = "PlayPause")]
    fn play_pause(&self) {
        self.calls.lock().expect("log").push("PlayPause".to_owned());
    }

    #[zbus(name = "Next")]
    fn next(&self) {
        self.calls.lock().expect("log").push("Next".to_owned());
    }

    #[zbus(name = "Previous")]
    fn previous(&self) {
        self.calls.lock().expect("log").push("Previous".to_owned());
    }
}

struct MockRoot {
    identity: String,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl MockRoot {
    #[zbus(property)]
    fn identity(&self) -> String {
        self.identity.clone()
    }

    #[zbus(property)]
    fn desktop_entry(&self) -> String {
        "mock-player".to_owned()
    }
}

/// Puts a player on the bus. `root` decides whether it serves the root
/// interface as well.
async fn serve_player(
    address: &str,
    name: &str,
    identity: &str,
    status: &str,
    root: bool,
) -> Arc<Mutex<Vec<String>>> {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let player = MockPlayer {
        status: status.to_owned(),
        calls: Arc::clone(&calls),
    };

    let mut builder = zbus::connection::Builder::address(address)
        .expect("the private bus address")
        .name(name)
        .expect("the well-known name")
        .serve_at(compass_media::PATH, player)
        .expect("serve the player interface");

    if root {
        builder = builder
            .serve_at(
                compass_media::PATH,
                MockRoot {
                    identity: identity.to_owned(),
                },
            )
            .expect("serve the root interface");
    }

    let connection = builder.build().await.expect("the mock player connects");
    std::mem::forget(connection);

    calls
}

async fn client(address: &str) -> MediaControl {
    let connection = zbus::connection::Builder::address(address)
        .expect("the private bus address")
        .build()
        .await
        .expect("the client connects");
    MediaControl::new(connection)
}

#[tokio::test]
async fn players_are_found_by_their_bus_names_and_nothing_else_is() {
    let Some(bus) = bus::start_or_skip("players_are_found_by_their_bus_names_and_nothing_else_is")
    else {
        return;
    };

    serve_player(
        bus.address(),
        "org.mpris.MediaPlayer2.mock",
        "Mock Player",
        "Playing",
        true,
    )
    .await;

    // Something else entirely, which must not be mistaken for a player.
    let other = zbus::connection::Builder::address(bus.address())
        .expect("address")
        .name("org.example.NotAPlayer")
        .expect("name")
        .build()
        .await
        .expect("connects");
    std::mem::forget(other);

    let names = client(bus.address())
        .await
        .player_names()
        .await
        .expect("list names");
    assert_eq!(names, vec!["org.mpris.MediaPlayer2.mock".to_owned()]);
}

#[tokio::test]
async fn a_player_is_assembled_from_both_interfaces() {
    let Some(bus) = bus::start_or_skip("a_player_is_assembled_from_both_interfaces") else {
        return;
    };
    serve_player(
        bus.address(),
        "org.mpris.MediaPlayer2.mock",
        "Mock Player",
        "Playing",
        true,
    )
    .await;

    let player = client(bus.address())
        .await
        .player("org.mpris.MediaPlayer2.mock")
        .await
        .expect("the player answers");

    assert_eq!(player.id, "org.mpris.MediaPlayer2.mock");
    assert_eq!(
        player.identity, "Mock Player",
        "Identity beats the bus name"
    );
    assert_eq!(player.app_id, "mock-player");
    assert_eq!(player.title, "A Song");
    assert_eq!(
        player.artist, "First, Second",
        "an artist array is joined with a comma and a space"
    );
    assert_eq!(player.status, PlaybackStatus::Playing);
    assert!(player.can_go_next);
    assert!(!player.can_go_previous);
}

#[tokio::test]
async fn a_player_without_the_root_interface_keeps_its_bus_name() {
    // The root call is optional in the C++, and a player that does not serve
    // it must still appear -- with a name, not a blank.
    let Some(bus) = bus::start_or_skip("a_player_without_the_root_interface_keeps_its_bus_name")
    else {
        return;
    };
    serve_player(
        bus.address(),
        "org.mpris.MediaPlayer2.bare",
        "ignored",
        "Paused",
        false,
    )
    .await;

    let player = client(bus.address())
        .await
        .player("org.mpris.MediaPlayer2.bare")
        .await
        .expect("the player answers its Player interface");

    assert_eq!(player.identity, "bare");
    assert_eq!(player.app_id, "");
    assert_eq!(player.status, PlaybackStatus::Paused);
}

#[tokio::test]
async fn a_player_that_does_not_answer_is_left_out_rather_than_failing_the_list() {
    // A launcher showing three of four players is better than one showing
    // none, and a departed player is the ordinary case of this.
    let Some(bus) = bus::start_or_skip(
        "a_player_that_does_not_answer_is_left_out_rather_than_failing_the_list",
    ) else {
        return;
    };
    serve_player(
        bus.address(),
        "org.mpris.MediaPlayer2.real",
        "Real",
        "Playing",
        true,
    )
    .await;

    // A name that looks like a player and serves nothing.
    let silent = zbus::connection::Builder::address(bus.address())
        .expect("address")
        .name("org.mpris.MediaPlayer2.silent")
        .expect("name")
        .build()
        .await
        .expect("connects");
    std::mem::forget(silent);

    let control = client(bus.address()).await;
    assert_eq!(control.player_names().await.expect("names").len(), 2);

    let players = control.players().await.expect("players");
    assert_eq!(players.len(), 1, "the silent name should have been skipped");
    assert_eq!(players[0].identity, "Real");
}

#[tokio::test]
async fn the_three_controls_reach_the_player_by_name() {
    let Some(bus) = bus::start_or_skip("the_three_controls_reach_the_player_by_name") else {
        return;
    };
    let calls = serve_player(
        bus.address(),
        "org.mpris.MediaPlayer2.mock",
        "Mock",
        "Playing",
        true,
    )
    .await;

    let control = client(bus.address()).await;
    control
        .play_pause("org.mpris.MediaPlayer2.mock")
        .await
        .expect("play/pause");
    control
        .next("org.mpris.MediaPlayer2.mock")
        .await
        .expect("next");
    control
        .previous("org.mpris.MediaPlayer2.mock")
        .await
        .expect("previous");

    assert_eq!(
        *calls.lock().expect("log"),
        vec![
            "PlayPause".to_owned(),
            "Next".to_owned(),
            "Previous".to_owned()
        ]
    );
}

#[tokio::test]
async fn controlling_a_player_that_is_gone_is_an_error() {
    let Some(bus) = bus::start_or_skip("controlling_a_player_that_is_gone_is_an_error") else {
        return;
    };
    let control = client(bus.address()).await;
    assert!(
        control
            .play_pause("org.mpris.MediaPlayer2.nobody")
            .await
            .is_err(),
        "a launcher must not report that it paused something that is not there"
    );
}
