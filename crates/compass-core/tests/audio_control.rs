//! What the audio service asks `pactl` for, and what it makes of the answer.
//!
//! Read off `PactlAudioControl` (`src/server/src/services/audio-control/`).

use std::cell::RefCell;

use compass_core::audio_control::{
    AudioSink, DEFAULT_SINK, DUMMY_ID, DummyAudioControl, PACTL_ID, Pactl, PactlAudioControl,
    TIMEOUT_MS,
};

/// A `pactl` that answers from a script and remembers what it was asked.
#[derive(Default)]
struct Fake {
    /// Replies, matched on the first argument; `None` means the call fails.
    replies: Vec<(&'static str, Option<String>)>,
    /// Every invocation, in order.
    ran: RefCell<Vec<Vec<String>>>,
}

impl Fake {
    /// A `pactl` that fails everything.
    fn failing() -> Self {
        Self::default()
    }

    /// A `pactl` answering `first_arg` with `output`.
    fn answering(mut self, first_arg: &'static str, output: &str) -> Self {
        self.replies.push((first_arg, Some(output.to_owned())));
        self
    }

    /// A `pactl` that reports one stereo sink, default and unmuted at 75%.
    fn with_one_sink() -> Self {
        Self::default()
            .answering("get-default-sink", "alsa_output.pci\n")
            .answering("--format=json", SINKS_JSON)
    }

    /// Every invocation, as joined argument strings.
    fn calls(&self) -> Vec<String> {
        self.ran
            .borrow()
            .iter()
            .map(|args| args.join(" "))
            .collect()
    }
}

impl Pactl for Fake {
    fn run(&self, args: &[&str]) -> Option<String> {
        self.ran
            .borrow_mut()
            .push(args.iter().map(|arg| (*arg).to_owned()).collect());
        self.replies
            .iter()
            .find(|(first, _)| args.first() == Some(first))
            .and_then(|(_, output)| output.clone())
    }
}

/// One default stereo sink, 75%, unmuted, headphones plugged in.
const SINKS_JSON: &str = r#"[
  {
    "name": "alsa_output.pci",
    "description": "Built-in Audio",
    "mute": false,
    "volume": {
      "front-left":  { "value_percent": "75%" },
      "front-right": { "value_percent": "75%" }
    },
    "ports": [
      { "name": "analog-output-speaker", "description": "Speakers" },
      { "name": "analog-output-headphones", "description": "Headphones" }
    ],
    "active_port": "analog-output-headphones"
  }
]"#;

#[test]
fn the_backend_names_itself_the_way_the_cpp_does() {
    assert_eq!(PactlAudioControl::new(Fake::failing()).id(), PACTL_ID);
    assert_eq!(PACTL_ID, "pactl");
    assert_eq!(DummyAudioControl.id(), DUMMY_ID);
    assert_eq!(DUMMY_ID, "dummy");
}

#[test]
fn the_timeout_is_three_seconds() {
    // A person pressing a volume key does not wait longer than this for the
    // command to be given up on.
    assert_eq!(TIMEOUT_MS, 3000);
}

#[test]
fn a_sink_is_read_out_of_the_json_the_way_the_cpp_reads_it() {
    let control = PactlAudioControl::new(Fake::with_one_sink());
    let sinks = control.list_sinks();
    assert_eq!(sinks.len(), 1);
    assert_eq!(
        sinks[0],
        AudioSink {
            name: "alsa_output.pci".to_owned(),
            description: "Built-in Audio".to_owned(),
            active_port: Some("Headphones".to_owned()),
            volume: 0.75,
            muted: false,
            is_default: true,
        }
    );
}

#[test]
fn the_active_port_is_reported_by_description_not_by_name() {
    // "Headphones" is what belongs on screen; "analog-output-headphones" is
    // not something to show a person.
    let control = PactlAudioControl::new(Fake::with_one_sink());
    assert_eq!(
        control.list_sinks()[0].active_port.as_deref(),
        Some("Headphones")
    );
}

#[test]
fn a_sink_whose_active_port_matches_nothing_has_no_port() {
    let json = SINKS_JSON.replace("analog-output-headphones\"\n", "nonexistent\"\n");
    let control = PactlAudioControl::new(
        Fake::default()
            .answering("get-default-sink", "alsa_output.pci")
            .answering("--format=json", &json),
    );
    assert_eq!(control.list_sinks()[0].active_port, None);
}

#[test]
fn the_default_sink_name_is_trimmed_of_its_newline() {
    // pactl prints it with a trailing newline. Comparing without trimming
    // would mark nothing as default, and every read below would then answer as
    // though there were no audio.
    let control = PactlAudioControl::new(Fake::with_one_sink());
    assert!(control.list_sinks()[0].is_default);
    assert!(control.default_sink().is_some());
}

#[test]
fn a_sink_that_is_not_the_default_is_not_marked_default() {
    let control = PactlAudioControl::new(
        Fake::default()
            .answering("get-default-sink", "some.other.sink\n")
            .answering("--format=json", SINKS_JSON),
    );
    assert!(!control.list_sinks()[0].is_default);
    assert_eq!(control.default_sink(), None);
}

#[test]
fn the_volume_comes_from_the_first_channel_by_name() {
    // The C++ reads volume.begin()->second out of a std::map, so it takes the
    // lexicographically first channel. Pinned with channels whose volumes
    // differ, which is the only arrangement that can tell the rule apart.
    let json = r#"[
      {
        "name": "s", "description": "S", "mute": false,
        "volume": {
          "front-right": { "value_percent": "20%" },
          "front-left":  { "value_percent": "80%" }
        },
        "ports": [], "active_port": ""
      }
    ]"#;
    let control = PactlAudioControl::new(
        Fake::default()
            .answering("get-default-sink", "s")
            .answering("--format=json", json),
    );
    assert!(
        (control.list_sinks()[0].volume - 0.8).abs() < f32::EPSILON,
        "front-left sorts first, so 80% is the answer"
    );
}

#[test]
fn a_sink_with_no_channels_reads_as_silent_rather_than_failing() {
    let json =
        r#"[{"name":"s","description":"S","mute":false,"volume":{},"ports":[],"active_port":""}]"#;
    let control = PactlAudioControl::new(
        Fake::default()
            .answering("get-default-sink", "s")
            .answering("--format=json", json),
    );
    assert_eq!(control.list_sinks()[0].volume, 0.0);
}

#[test]
fn a_volume_that_is_not_a_number_reads_as_zero_rather_than_crashing() {
    // The C++ calls std::stod, which throws on this, from inside a function
    // nobody catches around. See PARITY.md — a deliberate divergence.
    let json = r#"[{"name":"s","description":"S","mute":false,
      "volume":{"mono":{"value_percent":"n/a"}},"ports":[],"active_port":""}]"#;
    let control = PactlAudioControl::new(
        Fake::default()
            .answering("get-default-sink", "s")
            .answering("--format=json", json),
    );
    assert_eq!(control.list_sinks()[0].volume, 0.0);
}

#[test]
fn everything_is_empty_when_pactl_is_missing() {
    let control = PactlAudioControl::new(Fake::failing());
    assert!(control.list_sinks().is_empty());
    assert_eq!(control.default_sink(), None);
    assert_eq!(control.volume(), 0.0);
    assert!(!control.is_muted());
}

#[test]
fn a_failed_default_sink_lookup_stops_the_list_before_it_is_asked_for() {
    // Both commands have to succeed. A list where nothing is default would be
    // worse than no list, and it would also mean running the slower command
    // for nothing.
    let fake = Fake::default().answering("--format=json", SINKS_JSON);
    let control = PactlAudioControl::new(fake);
    assert!(control.list_sinks().is_empty());
    assert_eq!(
        control.pactl().calls(),
        vec!["get-default-sink".to_owned()],
        "the sink list must not be asked for once the default lookup failed"
    );
}

#[test]
fn unparseable_json_is_no_sinks_rather_than_a_guess() {
    let control = PactlAudioControl::new(
        Fake::default()
            .answering("get-default-sink", "s")
            .answering("--format=json", "not json"),
    );
    assert!(control.list_sinks().is_empty());
}

#[test]
fn setting_the_volume_asks_for_a_plain_percentage() {
    let control = PactlAudioControl::new(Fake::default().answering("set-sink-volume", ""));
    assert_eq!(control.set_volume(0.42), Some(0.42));
    assert_eq!(
        control.pactl().calls(),
        vec![format!("set-sink-volume {DEFAULT_SINK} 42%")],
        "no sign: this sets the volume, it does not nudge it"
    );
}

#[test]
fn setting_the_volume_clamps_and_says_what_it_clamped_to() {
    let control = PactlAudioControl::new(Fake::default().answering("set-sink-volume", ""));
    assert_eq!(control.set_volume(1.5), Some(1.0));
    assert_eq!(control.set_volume(-0.5), Some(0.0));
    assert_eq!(
        control.pactl().calls(),
        vec![
            format!("set-sink-volume {DEFAULT_SINK} 100%"),
            format!("set-sink-volume {DEFAULT_SINK} 0%"),
        ]
    );
}

#[test]
fn setting_the_volume_rounds_rather_than_truncating() {
    let control = PactlAudioControl::new(Fake::default().answering("set-sink-volume", ""));
    control.set_volume(0.666);
    assert_eq!(
        control.pactl().calls(),
        vec![format!("set-sink-volume {DEFAULT_SINK} 67%")]
    );
}

#[test]
fn setting_the_volume_reports_failure_rather_than_a_level_nobody_set() {
    let control = PactlAudioControl::new(Fake::failing());
    assert_eq!(control.set_volume(0.5), None);
}

#[test]
fn adjusting_the_volume_asks_for_a_signed_percentage() {
    // The sign is the whole difference between "set to 10%" and "turn it up by
    // 10%", and pactl tells them apart on exactly this character.
    let control = PactlAudioControl::new(
        Fake::default()
            .answering("set-sink-volume", "")
            .answering("get-default-sink", "alsa_output.pci\n")
            .answering("--format=json", SINKS_JSON),
    );
    control.adjust_volume(0.1);
    assert_eq!(
        control.pactl().calls()[0],
        format!("set-sink-volume {DEFAULT_SINK} +10%")
    );
}

#[test]
fn adjusting_downwards_keeps_its_minus_sign() {
    let control = PactlAudioControl::new(
        Fake::default()
            .answering("set-sink-volume", "")
            .answering("get-default-sink", "alsa_output.pci\n")
            .answering("--format=json", SINKS_JSON),
    );
    control.adjust_volume(-0.1);
    assert_eq!(
        control.pactl().calls()[0],
        format!("set-sink-volume {DEFAULT_SINK} -10%")
    );
}

#[test]
fn adjusting_reports_the_volume_it_read_back_not_the_one_it_asked_for() {
    // Something else may have moved the volume in between, so the arithmetic
    // answer would be a guess. The fake reports 75% whatever was asked.
    let control = PactlAudioControl::new(
        Fake::default()
            .answering("set-sink-volume", "")
            .answering("get-default-sink", "alsa_output.pci\n")
            .answering("--format=json", SINKS_JSON),
    );
    assert_eq!(control.adjust_volume(0.1), Some(0.75));
}

#[test]
fn adjusting_past_the_top_sets_it_back_to_one() {
    // pactl will go past 100%; the C++ reads back, notices, and sets 100%.
    let loud = SINKS_JSON.replace("75%", "150%");
    let control = PactlAudioControl::new(
        Fake::default()
            .answering("set-sink-volume", "")
            .answering("get-default-sink", "alsa_output.pci\n")
            .answering("--format=json", &loud),
    );
    assert_eq!(control.adjust_volume(0.5), Some(1.0));
    assert_eq!(
        control.pactl().calls().last().expect("a last call"),
        &format!("set-sink-volume {DEFAULT_SINK} 100%"),
        "the correction is a second command, not a clamp on the first"
    );
}

#[test]
fn adjusting_fails_without_reading_anything_back() {
    let control = PactlAudioControl::new(Fake::failing());
    assert_eq!(control.adjust_volume(0.1), None);
    assert_eq!(
        control.pactl().calls().len(),
        1,
        "a failed nudge must not go on to read the volume"
    );
}

#[test]
fn muting_and_unmuting_pass_one_and_zero() {
    let control = PactlAudioControl::new(Fake::default().answering("set-sink-mute", ""));
    assert!(control.set_muted(true));
    assert!(control.set_muted(false));
    assert_eq!(
        control.pactl().calls(),
        vec![
            format!("set-sink-mute {DEFAULT_SINK} 1"),
            format!("set-sink-mute {DEFAULT_SINK} 0"),
        ]
    );
}

#[test]
fn toggling_lets_pactl_decide_which_way() {
    // Not a read-then-write: doing it in two steps would race anything else
    // touching the volume.
    let control = PactlAudioControl::new(Fake::default().answering("set-sink-mute", ""));
    assert!(control.toggle_mute());
    assert_eq!(
        control.pactl().calls(),
        vec![format!("set-sink-mute {DEFAULT_SINK} toggle")]
    );
}

#[test]
fn a_mute_command_reports_whether_pactl_ran_not_the_resulting_state() {
    let control = PactlAudioControl::new(Fake::failing());
    assert!(!control.set_muted(true));
    assert!(!control.toggle_mute());
}

#[test]
fn a_muted_sink_reads_as_muted() {
    let muted = SINKS_JSON.replace("\"mute\": false", "\"mute\": true");
    let control = PactlAudioControl::new(
        Fake::default()
            .answering("get-default-sink", "alsa_output.pci\n")
            .answering("--format=json", &muted),
    );
    assert!(control.is_muted());
}

#[test]
fn setting_the_default_sink_names_it() {
    let control = PactlAudioControl::new(Fake::default().answering("set-default-sink", ""));
    assert!(control.set_default_sink("some.sink"));
    assert_eq!(
        control.pactl().calls(),
        vec!["set-default-sink some.sink".to_owned()]
    );
}

#[test]
fn the_dummy_backend_reads_flat_and_writes_nothing() {
    // A platform with no backend still has a provider, and everything it is
    // asked to do quietly fails. That is the contract, not an oversight.
    let dummy = DummyAudioControl;
    assert_eq!(dummy.volume(), 0.0);
    assert_eq!(dummy.set_volume(0.5), None);
    assert_eq!(dummy.adjust_volume(0.1), None);
    assert!(!dummy.is_muted());
    assert!(!dummy.set_muted(true));
    assert!(!dummy.toggle_mute());
}
