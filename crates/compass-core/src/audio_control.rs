//! Volume and mute, over `pactl`.
//!
//! A port of `AbstractAudioControl` and `PactlAudioControl`
//! (`src/server/src/services/audio-control/`), minus the process spawn.
//!
//! # What is worth testing here is the arguments
//!
//! Every operation is a `pactl` invocation and a little parsing. Running
//! `pactl` is not this module's business — it is one trait method — but
//! getting `+10%` where `10%` was meant is the difference between nudging the
//! volume and setting it, and there is no way to notice that from a type. So
//! the commands go through [`Pactl`] and the tests read them back.

use std::collections::BTreeMap;

use serde::Deserialize;

/// The identifier the Linux backend reports, from `PactlAudioControl::id`.
pub const PACTL_ID: &str = "pactl";

/// The identifier the fallback backend reports, from `DummyAudioControl::id`.
pub const DUMMY_ID: &str = "dummy";

/// The sink every command addresses: whatever is currently default.
pub const DEFAULT_SINK: &str = "@DEFAULT_SINK@";

/// How long the C++ waits for `pactl` before giving up, in milliseconds.
pub const TIMEOUT_MS: u64 = 3000;

/// Somewhere to run `pactl`.
pub trait Pactl {
    /// Run `pactl` with `args`, returning its standard output.
    ///
    /// `None` for every failure the C++ treats alike: the binary was not
    /// found, it timed out, or it exited non-zero.
    fn run(&self, args: &[&str]) -> Option<String>;
}

/// One output device, as the service reports it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AudioSink {
    /// The sink's name, which is also its id to `pactl`.
    pub name: String,
    /// What a person would call it.
    pub description: String,
    /// The *description* of the active port, not its name — the C++ copies
    /// `port.description`, because "Headphones" is what belongs on screen.
    pub active_port: Option<String>,
    /// The volume, as a fraction: `value_percent` over 100.
    pub volume: f32,
    /// Whether it is muted.
    pub muted: bool,
    /// Whether it is the default sink.
    pub is_default: bool,
}

/// One channel's volume, as `pactl --format=json` reports it.
#[derive(Debug, Clone, Deserialize)]
struct PactlVolume {
    /// A string like `"75%"`.
    value_percent: String,
}

/// One port on a sink.
#[derive(Debug, Clone, Deserialize)]
struct PactlPort {
    /// The port's id.
    name: String,
    /// What a person would call it.
    description: String,
}

/// One sink, as `pactl --format=json list sinks` reports it.
#[derive(Debug, Clone, Deserialize)]
struct PactlSink {
    /// The sink's id.
    name: String,
    /// What a person would call it.
    description: String,
    /// Whether it is muted.
    #[serde(default)]
    mute: bool,
    /// One entry per channel, keyed by channel name.
    ///
    /// A `BTreeMap` rather than a `HashMap` on purpose: the C++ reads
    /// `volume.begin()->second` out of a `std::map`, so it takes the
    /// *lexicographically first channel*. On a stereo sink that is
    /// `front-left`. Reading an arbitrary channel instead would give a sink
    /// with unbalanced channels a volume that changed between reads.
    #[serde(default)]
    volume: BTreeMap<String, PactlVolume>,
    /// Every port the sink has.
    #[serde(default)]
    ports: Vec<PactlPort>,
    /// Which port is in use, by name.
    #[serde(default)]
    active_port: String,
}

/// Turn one `pactl` sink into an [`AudioSink`], knowing which is default.
fn to_audio_sink(source: &PactlSink, default_name: &str) -> AudioSink {
    AudioSink {
        name: source.name.clone(),
        description: source.description.clone(),
        active_port: source
            .ports
            .iter()
            .find(|port| port.name == source.active_port)
            .map(|port| port.description.clone()),
        volume: source
            .volume
            .values()
            .next()
            .map_or(0.0, |volume| parse_percent(&volume.value_percent)),
        muted: source.mute,
        is_default: source.name == default_name,
    }
}

/// Read `"75%"` as `0.75`.
///
/// # A divergence, in the safe direction
///
/// The C++ calls `std::stod`, which parses the leading number and ignores the
/// `%` — and *throws* on a string with no leading number at all, from inside a
/// function nobody catches around. A `pactl` that ever printed `"n/a"` would
/// take the process down. This returns 0.0 instead, which is what the sink
/// already defaults to when the channel map is empty.
fn parse_percent(text: &str) -> f32 {
    let digits: String = text
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+')
        .collect();
    digits.parse::<f32>().unwrap_or(0.0) / 100.0
}

/// Strip trailing newlines, as the C++ does to `get-default-sink`'s output.
fn trim_trailing_newlines(text: &str) -> &str {
    text.trim_end_matches('\n')
}

/// Volume and mute over `pactl`.
#[derive(Debug)]
pub struct PactlAudioControl<P> {
    /// Where commands run.
    pactl: P,
}

impl<P: Pactl> PactlAudioControl<P> {
    /// Control audio through `pactl`.
    pub const fn new(pactl: P) -> Self {
        Self { pactl }
    }

    /// What this backend calls itself.
    #[must_use]
    pub const fn id(&self) -> &'static str {
        PACTL_ID
    }

    /// The runner, for a caller that has to read back what was run.
    pub const fn pactl(&self) -> &P {
        &self.pactl
    }

    /// Every sink `pactl` knows.
    ///
    /// Empty when either command fails — including when only the default-sink
    /// lookup does, since a sink list with nothing marked default would make
    /// every read below answer as though there were no audio at all, which is
    /// a more confusing lie than "no sinks".
    #[must_use]
    pub fn list_sinks(&self) -> Vec<AudioSink> {
        let Some(default_name) = self.pactl.run(&["get-default-sink"]) else {
            return Vec::new();
        };
        let Some(json) = self.pactl.run(&["--format=json", "list", "sinks"]) else {
            return Vec::new();
        };
        let Ok(sinks) = serde_json::from_str::<Vec<PactlSink>>(&json) else {
            // The C++ warns and returns {}; an unparseable list is not an
            // excuse to guess at a volume.
            return Vec::new();
        };

        let default_name = trim_trailing_newlines(&default_name);
        sinks
            .iter()
            .map(|sink| to_audio_sink(sink, default_name))
            .collect()
    }

    /// The default sink, if there is one.
    #[must_use]
    pub fn default_sink(&self) -> Option<AudioSink> {
        self.list_sinks().into_iter().find(|sink| sink.is_default)
    }

    /// Make `sink_name` the default.
    pub fn set_default_sink(&self, sink_name: &str) -> bool {
        self.pactl.run(&["set-default-sink", sink_name]).is_some()
    }

    /// The default sink's volume, or 0 when there is no default sink.
    #[must_use]
    pub fn volume(&self) -> f32 {
        self.default_sink().map_or(0.0, |sink| sink.volume)
    }

    /// Set the volume to `level`, clamped to 0..=1.
    ///
    /// Returns the level that was set, or `None` if `pactl` failed. The level
    /// returned is the *clamped* one, so a caller asking for 1.5 is told 1.0
    /// rather than being left to believe it went to 150%.
    pub fn set_volume(&self, level: f32) -> Option<f32> {
        let level = level.clamp(0.0, 1.0);
        let percent = format!("{}%", (level * 100.0).round() as i32);
        self.pactl
            .run(&["set-sink-volume", DEFAULT_SINK, &percent])?;
        Some(level)
    }

    /// Change the volume by `delta`, which may be negative.
    ///
    /// # It reads back, and can still exceed 1.0
    ///
    /// `pactl` will happily go past 100%, so after nudging, the C++ reads the
    /// volume and, if it is over 1.0, sets it *back* to 1.0. The value
    /// returned is therefore the real volume rather than the arithmetic one,
    /// and a caller that computed it themselves would be wrong whenever
    /// something else moved the volume in between.
    pub fn adjust_volume(&self, delta: f32) -> Option<f32> {
        let percent = format!("{:+}%", (delta * 100.0).round() as i32);
        self.pactl
            .run(&["set-sink-volume", DEFAULT_SINK, &percent])?;

        let volume = self.volume();
        if volume > 1.0 {
            return self.set_volume(1.0);
        }
        Some(volume)
    }

    /// Whether the default sink is muted; false when there is no default sink.
    #[must_use]
    pub fn is_muted(&self) -> bool {
        self.default_sink().is_some_and(|sink| sink.muted)
    }

    /// Mute or unmute.
    ///
    /// The returned bool is whether `pactl` *ran*, not whether the sink ended
    /// up muted — which is what the C++ returns, and what a caller reading it
    /// as "it worked" gets.
    pub fn set_muted(&self, muted: bool) -> bool {
        let value = if muted { "1" } else { "0" };
        self.pactl
            .run(&["set-sink-mute", DEFAULT_SINK, value])
            .is_some()
    }

    /// Flip the mute, letting `pactl` decide which way.
    pub fn toggle_mute(&self) -> bool {
        self.pactl
            .run(&["set-sink-mute", DEFAULT_SINK, "toggle"])
            .is_some()
    }
}

/// The backend on a platform with none: every read is flat and every write
/// fails.
///
/// `DummyAudioControl` exists so the service always has a provider. Reproduced
/// so that a caller written against it behaves the same here — silently doing
/// nothing is the contract, not an oversight.
#[derive(Debug, Clone, Copy, Default)]
pub struct DummyAudioControl;

impl DummyAudioControl {
    /// What this backend calls itself.
    #[must_use]
    pub const fn id(&self) -> &'static str {
        DUMMY_ID
    }

    /// Always 0.
    #[must_use]
    pub const fn volume(&self) -> f32 {
        0.0
    }

    /// Always fails.
    #[must_use]
    pub const fn set_volume(&self, _level: f32) -> Option<f32> {
        None
    }

    /// Always fails.
    #[must_use]
    pub const fn adjust_volume(&self, _delta: f32) -> Option<f32> {
        None
    }

    /// Always false.
    #[must_use]
    pub const fn is_muted(&self) -> bool {
        false
    }

    /// Always fails.
    #[must_use]
    pub const fn set_muted(&self, _muted: bool) -> bool {
        false
    }

    /// Always fails.
    #[must_use]
    pub const fn toggle_mute(&self) -> bool {
        false
    }
}
