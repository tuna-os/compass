//! The media and volume commands.
//!
//! Ports `src/server/src/builtins/media/` — which commands exist on which
//! platform, how a player is chosen from what the user typed, what the
//! on-screen display says, and which speaker glyph goes with a volume.

/// The default step, in percent, for turning the volume up.
pub const VOLUME_UP_STEP: i32 = 5;

/// The default step, in percent, for turning the volume down.
pub const VOLUME_DOWN_STEP: i32 = -5;

/// The volume presets, as (percent, icon).
///
/// The icons are not what [`volume_icon`] would choose: 50% is listed with the
/// *low* speaker while `volume_icon(0.5)` returns the *down* one. The preset
/// list is written out by hand in the C++ and drifted from the function; both
/// are ported as they are, because changing either would change what someone
/// sees today and neither is more right than the other.
pub const VOLUME_PRESETS: &[(i32, &str)] = &[
    (100, "speaker-high"),
    (75, "speaker-high"),
    (50, "speaker-low"),
    (25, "speaker-low"),
    (0, "speaker-off"),
];

/// The commands the media extension registers.
///
/// The player commands need MPRIS, which exists on Linux and Windows; the
/// volume commands go through the audio service and are registered everywhere.
/// On a platform with no MPRIS the player commands are absent rather than
/// present and failing.
#[must_use]
pub fn registered_commands(has_media_control: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if has_media_control {
        out.extend(
            ["now-playing", "play-pause", "next-track", "previous-track"]
                .iter()
                .map(|s| (*s).to_owned()),
        );
    }
    out.push("volume-up".to_owned());
    out.push("volume-down".to_owned());
    for (percent, _) in VOLUME_PRESETS {
        out.push(format!("volume-{percent}"));
    }
    out.push("toggle-mute".to_owned());
    out
}

/// A running media player, as the commands see it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaPlayer {
    /// The bus name the player is addressed by.
    pub id: String,
    /// What the player calls itself.
    pub identity: String,
    /// The current track's title.
    pub title: String,
    /// The current track's artist.
    pub artist: String,
    /// Whether it is playing right now.
    pub playing: bool,
    /// Whether it has a next track.
    pub can_go_next: bool,
    /// Whether it has a previous track.
    pub can_go_previous: bool,
}

/// The fields a player is fuzzy-matched on, weighted as the C++'s
/// `FuzzySearchable<MediaPlayer>`: title 1.0, artist 0.8, identity 0.6.
impl compass_search::FuzzySearchable for MediaPlayer {
    fn fuzzy_fields<'a>(&'a self, out: &mut Vec<compass_search::WeightedField<'a>>) {
        out.push(compass_search::WeightedField::new(&self.title, 1.0));
        out.push(compass_search::WeightedField::new(&self.artist, 0.8));
        out.push(compass_search::WeightedField::new(&self.identity, 0.6));
    }
}

/// The players `query` matches, best first, as indices into `players`: what
/// [`resolve_player`] takes as its `matches`.
#[must_use]
pub fn player_matches(query: &str, players: &[MediaPlayer]) -> Vec<usize> {
    compass_search::rank_indices(query, players)
        .into_iter()
        .map(|scored| scored.item)
        .collect()
}

/// The optional argument a media command takes, as `(name, placeholder)`:
/// `player` for the three player commands, `step` for the two volume nudges,
/// none for the rest.
#[must_use]
pub fn command_argument(id: &str) -> Option<(&'static str, &'static str)> {
    match id {
        "play-pause" | "next-track" | "previous-track" => Some(("player", "player")),
        "volume-up" => Some(("step", "+5")),
        "volume-down" => Some(("step", "-5")),
        _ => None,
    }
}

/// How a player is named in the on-screen display.
///
/// Three cases, narrowing: a player with no track falls back to its own name,
/// a track with no artist is named by its title alone, and only a complete
/// track gets `title — artist`. The separator is an em dash with spaces around
/// it, which is the C++'s `%1 — %2`.
#[must_use]
pub fn track_label(player: &MediaPlayer) -> String {
    if player.title.is_empty() {
        return player.identity.clone();
    }
    if player.artist.is_empty() {
        return player.title.clone();
    }
    format!("{} — {}", player.title, player.artist)
}

/// Why no player could be acted on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoPlayer {
    /// Nothing is running at all.
    NothingRunning,
    /// Something is running, but nothing matched what was typed.
    NoMatch(String),
}

/// The message shown when no player could be acted on.
///
/// The two are different sentences on purpose. "No media player is running" is
/// a fact about the system; "No media player matches …" is a fact about what
/// was typed, and quoting the query back is what tells someone they misspelled
/// it rather than that their music stopped.
#[must_use]
pub fn no_player_message(reason: &NoPlayer) -> String {
    match reason {
        NoPlayer::NothingRunning => "No media player is running".to_owned(),
        NoPlayer::NoMatch(query) => format!("No media player matches \"{query}\""),
    }
}

/// Which player a command acts on.
///
/// An empty query takes the default player — the one the media service already
/// considers active — rather than searching. Anything else is a fuzzy search
/// over the running players, and the best match is taken.
pub fn resolve_player(
    query: &str,
    default_player: Option<usize>,
    matches: &[usize],
) -> Result<usize, NoPlayer> {
    if query.is_empty() {
        return default_player.ok_or(NoPlayer::NothingRunning);
    }
    matches
        .first()
        .copied()
        .ok_or_else(|| NoPlayer::NoMatch(query.to_owned()))
}

/// What the display says after a play/pause.
///
/// The state tested is the one from *before* the toggle, so a player that was
/// playing reports `Paused`. Reading the state back after the call would race
/// the player's own reply.
#[must_use]
pub fn play_pause_message(player: &MediaPlayer) -> String {
    if player.playing {
        "Paused".to_owned()
    } else {
        format!("Playing {}", track_label(player))
    }
}

/// Whether a skip is possible, and what to say when it is not.
///
/// The player is asked before the call rather than after it fails, so the
/// message can name the player instead of reporting a failure with no cause.
#[must_use]
pub fn skip_refusal(player: &MediaPlayer, forward: bool) -> Option<String> {
    let able = if forward {
        player.can_go_next
    } else {
        player.can_go_previous
    };
    if able {
        return None;
    }
    let direction = if forward { "next" } else { "previous" };
    Some(format!(
        "{} cannot skip to the {direction} track",
        player.identity
    ))
}

/// The speaker glyph for a volume level.
///
/// Four bands, and the boundaries belong to the *lower* band: exactly 0.33 is
/// low, not down. Silence is its own case rather than the bottom of the first
/// band, so a muted system shows a crossed-out speaker and not a quiet one.
#[must_use]
pub fn volume_icon(level: f32) -> &'static str {
    if level <= 0.0 {
        "speaker-off"
    } else if level <= 0.33 {
        "speaker-low"
    } else if level <= 0.66 {
        "speaker-down"
    } else {
        "speaker-high"
    }
}

/// What the on-screen display says for a volume.
///
/// The level is a fraction and the display is a percentage, rounded half away
/// from zero the way `qRound` does — so 0.005 shows as 1% rather than 0%, and
/// a nudge that changed something never reads as if it changed nothing.
#[must_use]
pub fn volume_hud_text(level: f32) -> String {
    format!("Volume {}%", round_half_away_from_zero(level * 100.0))
}

/// `qRound`'s rounding: halves go away from zero, not to even.
#[must_use]
pub fn round_half_away_from_zero(value: f32) -> i32 {
    if value < 0.0 {
        -((-value + 0.5).floor() as i32)
    } else {
        (value + 0.5).floor() as i32
    }
}

/// How far to move the volume, from what the user typed.
///
/// An absent or empty argument takes the command's default. Anything that is
/// not an integer is refused rather than silently treated as the default,
/// because a typo in a step is more likely than a deliberate `+five`.
///
/// The sign is **not** forced to match the command. `volume-down 5` turns the
/// volume *up* by five, because both commands share one `adjustVolume` call
/// and neither negates what it was given. That is the C++'s behaviour and the
/// port keeps it; the defaults are what carry the direction.
pub fn volume_step(argument: Option<&str>, default_step: i32) -> Result<i32, &'static str> {
    match argument {
        None | Some("") => Ok(default_step),
        Some(text) => text.parse::<i32>().map_err(|_| "Invalid step value"),
    }
}

/// The fraction `adjustVolume` is called with, for a step in percent.
#[must_use]
pub fn step_fraction(step: i32) -> f32 {
    step as f32 / 100.0
}

/// What the display says after a mute toggle.
///
/// A muted system says so; an unmuted one shows the volume it came back to,
/// which is the number someone needs and `Unmuted` would not give them.
#[must_use]
pub fn mute_message(muted: bool, level: f32) -> String {
    if muted {
        "Muted".to_owned()
    } else {
        volume_hud_text(level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_presets_cover_five_steps_and_registered_commands() {
        assert_eq!(VOLUME_PRESETS.len(), 5);
        let cmds = registered_commands(false);
        assert!(!cmds.contains(&"now-playing".to_owned()));
        assert!(cmds.contains(&"volume-up".to_owned()));
        assert!(cmds.contains(&"volume-100".to_owned()));
        assert!(cmds.contains(&"toggle-mute".to_owned()));
        let with_media = registered_commands(true);
        assert!(with_media.contains(&"now-playing".to_owned()));
        assert!(with_media.contains(&"play-pause".to_owned()));
    }

    #[test]
    fn track_label_falls_back_correctly() {
        let empty = MediaPlayer {
            identity: "Spotify".to_owned(),
            ..Default::default()
        };
        assert_eq!(track_label(&empty), "Spotify");
        let titled = MediaPlayer {
            title: "Song".to_owned(),
            identity: "Spotify".to_owned(),
            ..Default::default()
        };
        assert_eq!(track_label(&titled), "Song");
        let full = MediaPlayer {
            title: "Song".to_owned(),
            artist: "Artist".to_owned(),
            identity: "Spotify".to_owned(),
            ..Default::default()
        };
        assert_eq!(track_label(&full), "Song — Artist");
    }

    #[test]
    fn no_player_message_quotes_query() {
        assert_eq!(
            no_player_message(&NoPlayer::NothingRunning),
            "No media player is running"
        );
        assert_eq!(
            no_player_message(&NoPlayer::NoMatch("foo".to_owned())),
            "No media player matches \"foo\""
        );
    }

    #[test]
    fn resolve_player_default_vs_search() {
        assert_eq!(resolve_player("", Some(0), &[]), Ok(0));
        assert_eq!(resolve_player("", None, &[]), Err(NoPlayer::NothingRunning));
        assert_eq!(resolve_player("spot", None, &[2]), Ok(2));
        assert_eq!(
            resolve_player("spot", None, &[]),
            Err(NoPlayer::NoMatch("spot".to_owned()))
        );
    }

    #[test]
    fn play_pause_and_skip_messages() {
        let playing = MediaPlayer {
            playing: true,
            title: "T".to_owned(),
            artist: "A".to_owned(),
            identity: "Spotify".to_owned(),
            ..Default::default()
        };
        assert_eq!(play_pause_message(&playing), "Paused");
        let paused = MediaPlayer {
            playing: false,
            ..playing.clone()
        };
        assert!(play_pause_message(&paused).starts_with("Playing"));
        let cannot = MediaPlayer {
            can_go_next: false,
            can_go_previous: true,
            identity: "Spotify".to_owned(),
            ..Default::default()
        };
        assert!(skip_refusal(&cannot, true).is_some());
        assert!(skip_refusal(&cannot, false).is_none());
    }

    #[test]
    fn volume_icon_bands_and_hud_rounding() {
        assert_eq!(volume_icon(0.0), "speaker-off");
        assert_eq!(volume_icon(0.33), "speaker-low");
        assert_eq!(volume_icon(0.34), "speaker-down");
        assert_eq!(volume_icon(0.67), "speaker-high");
        assert_eq!(volume_hud_text(0.005), "Volume 1%");
        assert_eq!(volume_hud_text(0.5), "Volume 50%");
        assert_eq!(round_half_away_from_zero(0.5), 1);
        assert_eq!(round_half_away_from_zero(-0.5), -1);
        assert_eq!(round_half_away_from_zero(1.5), 2);
    }

    #[test]
    fn a_player_is_matched_on_its_track_then_its_name() {
        let players = [
            MediaPlayer {
                id: "org.mpris.MediaPlayer2.firefox".to_owned(),
                identity: "Firefox".to_owned(),
                title: "A lecture".to_owned(),
                ..Default::default()
            },
            MediaPlayer {
                id: "org.mpris.MediaPlayer2.spotify".to_owned(),
                identity: "Spotify".to_owned(),
                title: "Blue Monday".to_owned(),
                artist: "New Order".to_owned(),
                ..Default::default()
            },
        ];
        assert_eq!(player_matches("spot", &players).first(), Some(&1));
        assert_eq!(player_matches("lecture", &players).first(), Some(&0));
        assert_eq!(player_matches("new order", &players).first(), Some(&1));
        assert!(player_matches("vlc", &players).is_empty());
        assert_eq!(command_argument("play-pause"), Some(("player", "player")));
        assert_eq!(command_argument("volume-down"), Some(("step", "-5")));
        assert_eq!(command_argument("volume-50"), None);
    }

    #[test]
    fn volume_step_parses_or_defaults() {
        assert_eq!(volume_step(None, 5), Ok(5));
        assert_eq!(volume_step(Some(""), 5), Ok(5));
        assert_eq!(volume_step(Some("10"), 5), Ok(10));
        assert!(volume_step(Some("five"), 5).is_err());
        // sign not forced
        assert_eq!(volume_step(Some("5"), -5), Ok(5));
        assert_eq!(step_fraction(5), 0.05);
        assert_eq!(mute_message(true, 0.5), "Muted");
        assert_eq!(mute_message(false, 0.5), "Volume 50%");
    }
}
