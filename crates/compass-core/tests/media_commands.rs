//! The media and volume commands.
//!
//! Ported from `src/server/src/builtins/media/`.

use compass_core::media_commands::{
    MediaPlayer, NoPlayer, VOLUME_DOWN_STEP, VOLUME_PRESETS, VOLUME_UP_STEP, mute_message,
    no_player_message, play_pause_message, registered_commands, resolve_player,
    round_half_away_from_zero, skip_refusal, step_fraction, track_label, volume_hud_text,
    volume_icon, volume_step,
};

fn player() -> MediaPlayer {
    MediaPlayer {
        id: "org.mpris.MediaPlayer2.spotify".to_owned(),
        identity: "Spotify".to_owned(),
        title: "Blue Monday".to_owned(),
        artist: "New Order".to_owned(),
        playing: true,
        can_go_next: true,
        can_go_previous: true,
    }
}

// --- which commands exist -----------------------------------------------

#[test]
fn a_platform_with_no_mpris_still_gets_the_volume_commands() {
    // They go through the audio service, not through a media player, so they
    // are absent only where there is no audio at all.
    let commands = registered_commands(false);
    assert!(!commands.iter().any(|c| c == "play-pause"));
    assert!(commands.iter().any(|c| c == "volume-up"));
    assert!(commands.iter().any(|c| c == "toggle-mute"));
}

#[test]
fn the_player_commands_appear_where_mpris_does() {
    // Absent rather than present-and-failing.
    let commands = registered_commands(true);
    for id in ["now-playing", "play-pause", "next-track", "previous-track"] {
        assert!(commands.iter().any(|c| c == id), "missing {id}");
    }
}

#[test]
fn every_preset_gets_a_command_named_after_its_percentage() {
    let commands = registered_commands(false);
    for (percent, _) in VOLUME_PRESETS {
        assert!(
            commands.iter().any(|c| c == &format!("volume-{percent}")),
            "missing volume-{percent}"
        );
    }
}

#[test]
fn the_presets_are_these_five() {
    // Pinned as a list rather than iterated, because every other test here
    // walks `VOLUME_PRESETS` and would follow it wherever it went.
    assert_eq!(
        VOLUME_PRESETS,
        [
            (100, "speaker-high"),
            (75, "speaker-high"),
            (50, "speaker-low"),
            (25, "speaker-low"),
            (0, "speaker-off"),
        ]
    );
}

#[test]
fn silence_is_one_of_the_presets() {
    // `volume-0` is a preset, not the bottom of the volume-down command.
    assert!(VOLUME_PRESETS.iter().any(|(p, _)| *p == 0));
}

#[test]
fn the_preset_icons_do_not_agree_with_the_icon_function() {
    // 50% is listed with the low speaker while `volume_icon(0.5)` gives the
    // down one. The list drifted from the function in the C++; both are
    // ported as they are, because changing either changes what someone sees
    // today and neither is more right.
    let fifty = VOLUME_PRESETS
        .iter()
        .find(|(p, _)| *p == 50)
        .expect("a 50% preset");
    assert_eq!(fifty.1, "speaker-low");
    assert_eq!(volume_icon(0.5), "speaker-down");
}

// --- naming a player ----------------------------------------------------

#[test]
fn a_complete_track_is_named_by_title_and_artist() {
    assert_eq!(track_label(&player()), "Blue Monday — New Order");
}

#[test]
fn the_separator_is_an_em_dash_with_spaces() {
    assert!(track_label(&player()).contains(" — "));
}

#[test]
fn a_track_with_no_artist_is_named_by_its_title_alone() {
    let p = MediaPlayer {
        artist: String::new(),
        ..player()
    };
    assert_eq!(track_label(&p), "Blue Monday");
}

#[test]
fn a_player_with_no_track_falls_back_to_its_own_name() {
    let p = MediaPlayer {
        title: String::new(),
        ..player()
    };
    assert_eq!(track_label(&p), "Spotify");
}

#[test]
fn a_player_with_an_artist_but_no_title_still_falls_back_to_its_name() {
    // The title is tested first, so an artist with nothing to attribute does
    // not produce a label reading " — New Order".
    let p = MediaPlayer {
        title: String::new(),
        ..player()
    };
    assert_eq!(track_label(&p), "Spotify");
}

#[test]
fn a_player_with_neither_title_nor_artist_is_named_by_itself() {
    // This is the only case where the order of the two checks shows. With one
    // of the pair present either order gives the same answer; with both empty,
    // testing the artist first would return the empty title and leave the row
    // blank.
    let p = MediaPlayer {
        title: String::new(),
        artist: String::new(),
        ..player()
    };
    assert_eq!(track_label(&p), "Spotify");
}

// --- choosing a player --------------------------------------------------

#[test]
fn an_empty_query_takes_the_default_player() {
    // The one the media service already considers active, rather than a search
    // that happens to match everything.
    assert_eq!(resolve_player("", Some(2), &[0, 1]), Ok(2));
}

#[test]
fn an_empty_query_with_nothing_running_reports_nothing_running() {
    assert_eq!(resolve_player("", None, &[]), Err(NoPlayer::NothingRunning));
}

#[test]
fn a_query_takes_the_best_match_and_not_the_default() {
    assert_eq!(resolve_player("spot", Some(9), &[4, 7]), Ok(4));
}

#[test]
fn a_query_that_matches_nothing_says_so_and_quotes_it_back() {
    // "No media player is running" is a fact about the system; this is a fact
    // about what was typed, and quoting it is what says "you misspelled it"
    // rather than "your music stopped".
    let err = resolve_player("spot", Some(9), &[]).unwrap_err();
    assert_eq!(err, NoPlayer::NoMatch("spot".to_owned()));
    assert_eq!(no_player_message(&err), "No media player matches \"spot\"");
}

#[test]
fn the_two_failures_are_different_sentences() {
    assert_eq!(
        no_player_message(&NoPlayer::NothingRunning),
        "No media player is running"
    );
    assert_ne!(
        no_player_message(&NoPlayer::NothingRunning),
        no_player_message(&NoPlayer::NoMatch(String::new()))
    );
}

// --- playback -----------------------------------------------------------

#[test]
fn pausing_a_playing_player_says_paused() {
    // The state tested is the one from before the toggle; reading it back
    // afterwards would race the player's own reply.
    assert_eq!(play_pause_message(&player()), "Paused");
}

#[test]
fn resuming_names_the_track() {
    let p = MediaPlayer {
        playing: false,
        ..player()
    };
    assert_eq!(play_pause_message(&p), "Playing Blue Monday — New Order");
}

#[test]
fn resuming_a_player_with_no_track_names_the_player() {
    let p = MediaPlayer {
        playing: false,
        title: String::new(),
        ..player()
    };
    assert_eq!(play_pause_message(&p), "Playing Spotify");
}

#[test]
fn a_player_that_can_skip_raises_no_objection() {
    assert_eq!(skip_refusal(&player(), true), None);
    assert_eq!(skip_refusal(&player(), false), None);
}

#[test]
fn a_refusal_names_the_player_and_the_direction() {
    // Asked before the call rather than after it fails, so the message has a
    // cause in it instead of just a failure.
    let p = MediaPlayer {
        can_go_next: false,
        ..player()
    };
    assert_eq!(
        skip_refusal(&p, true).as_deref(),
        Some("Spotify cannot skip to the next track")
    );
    assert_eq!(skip_refusal(&p, false), None);
}

#[test]
fn the_two_directions_are_checked_independently() {
    let p = MediaPlayer {
        can_go_previous: false,
        ..player()
    };
    assert_eq!(skip_refusal(&p, true), None);
    assert_eq!(
        skip_refusal(&p, false).as_deref(),
        Some("Spotify cannot skip to the previous track")
    );
}

// --- volume glyphs ------------------------------------------------------

#[test]
fn silence_gets_the_crossed_out_speaker() {
    // Its own case rather than the bottom of the first band, so a muted system
    // does not show a quiet one.
    assert_eq!(volume_icon(0.0), "speaker-off");
}

#[test]
fn the_four_bands_are_distinct() {
    assert_eq!(volume_icon(0.1), "speaker-low");
    assert_eq!(volume_icon(0.5), "speaker-down");
    assert_eq!(volume_icon(0.9), "speaker-high");
}

#[test]
fn a_band_boundary_belongs_to_the_lower_band() {
    assert_eq!(volume_icon(0.33), "speaker-low");
    assert_eq!(volume_icon(0.66), "speaker-down");
}

#[test]
fn just_past_a_boundary_is_the_next_band() {
    assert_eq!(volume_icon(0.34), "speaker-down");
    assert_eq!(volume_icon(0.67), "speaker-high");
}

// --- the volume display -------------------------------------------------

#[test]
fn the_display_shows_a_percentage_not_a_fraction() {
    assert_eq!(volume_hud_text(0.42), "Volume 42%");
}

#[test]
fn a_tiny_volume_rounds_up_rather_than_reading_as_silence() {
    // A nudge that changed something should not read as if it changed nothing.
    assert_eq!(volume_hud_text(0.005), "Volume 1%");
}

#[test]
fn halves_go_away_from_zero_the_way_qround_does() {
    assert_eq!(round_half_away_from_zero(0.5), 1);
    assert_eq!(round_half_away_from_zero(1.5), 2);
    assert_eq!(round_half_away_from_zero(2.5), 3);
    assert_eq!(round_half_away_from_zero(-0.5), -1);
}

#[test]
fn rounding_is_not_to_even() {
    // Banker's rounding would make 2.5 into 2.
    assert_ne!(round_half_away_from_zero(2.5), 2);
}

// --- the volume step ----------------------------------------------------

#[test]
fn an_absent_argument_takes_the_commands_default() {
    assert_eq!(volume_step(None, VOLUME_UP_STEP), Ok(5));
    assert_eq!(volume_step(None, VOLUME_DOWN_STEP), Ok(-5));
}

#[test]
fn an_empty_argument_also_takes_the_default() {
    assert_eq!(volume_step(Some(""), VOLUME_UP_STEP), Ok(5));
}

#[test]
fn a_number_replaces_the_default() {
    assert_eq!(volume_step(Some("20"), VOLUME_UP_STEP), Ok(20));
}

#[test]
fn a_signed_number_is_accepted() {
    assert_eq!(volume_step(Some("-20"), VOLUME_UP_STEP), Ok(-20));
}

#[test]
fn a_step_that_is_not_a_number_is_refused_rather_than_defaulted() {
    // A typo in a step is more likely than a deliberate `+five`, so it is
    // reported instead of silently doing something else.
    assert_eq!(
        volume_step(Some("five"), VOLUME_UP_STEP),
        Err("Invalid step value")
    );
    assert_eq!(
        volume_step(Some("5abc"), VOLUME_UP_STEP),
        Err("Invalid step value")
    );
}

#[test]
fn turning_the_volume_down_by_a_positive_step_turns_it_up() {
    // Both commands share one `adjustVolume` call and neither negates what it
    // was given, so the defaults are the only thing carrying the direction.
    // This is the C++'s behaviour, kept rather than quietly corrected.
    assert_eq!(volume_step(Some("5"), VOLUME_DOWN_STEP), Ok(5));
}

#[test]
fn a_step_in_percent_becomes_a_fraction() {
    assert!((step_fraction(5) - 0.05).abs() < 1e-6);
    assert!((step_fraction(-5) + 0.05).abs() < 1e-6);
}

// --- mute ---------------------------------------------------------------

#[test]
fn muting_says_muted() {
    assert_eq!(mute_message(true, 0.42), "Muted");
}

#[test]
fn unmuting_shows_the_volume_it_came_back_to() {
    // The number someone needs, which `Unmuted` would not give them.
    assert_eq!(mute_message(false, 0.42), "Volume 42%");
}
