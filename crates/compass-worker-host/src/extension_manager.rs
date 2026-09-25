//! The `Manager` service, typed.
//!
//! [`crate::rpc`] knows the envelope; this knows what goes inside it for the
//! one service the host talks to. Both sides of that conversation are
//! generated from `figura/manager.fig` — the worker's by the figura generator
//! (`crates/compass-figura`), the C++ engine's by its glaze generator until
//! that engine was removed — so this is another implementation of one IDL, and
//! the tests below read the IDL rather than restating it.
//!
//! # Two details that are not guessable and were read out of the generator
//!
//! **Params are a named object keyed by parameter name.** The codegen emits
//! `this.transport.request("Manager/load", { opts })` for `fn load(opts:
//! LoadOptions)`, and the server side routes `msg.params.opts` back into it.
//! So `load`'s params are `{"opts": {…}}`, not the `LoadOptions` object itself
//! and not a positional array. Getting this wrong produces a worker that
//! answers with `undefined` fields rather than an error.
//!
//! **The field names are the IDL's, inconsistency included.** `LoadOptions`
//! is snake_case apart from `fallbackText`, and `Capabilities` is camelCase
//! throughout. Rust spells its fields the way Rust spells fields and renames
//! them on the way out; `the_field_names_match_the_idl` serialises a value and
//! compares the resulting keys to the IDL, so a rename that drops or tidies one
//! fails there rather than at a worker that sees `undefined`.

use serde::{Deserialize, Serialize};

use crate::{Worker, WorkerError, rpc};

/// Whether the command draws a view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandMode {
    /// A command with a UI.
    View,
    /// A command that runs and exits without one.
    NoView,
}

/// Which build of the worker runtime a command needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandEnv {
    /// A development extension; the worker is created on the fly with the
    /// development React build.
    Development,
    /// Everything else; taken from the worker pool.
    Production,
}

/// What caused the command to be launched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LaunchType {
    /// A person asked for it.
    User,
    /// A scheduled or background run.
    Background,
    /// The CLI asked for it.
    CommandLine,
}

/// The host facilities an extension is allowed to reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools, reason = "the IDL's shape, not ours")]
pub struct Capabilities {
    /// The browser extension bridge.
    #[serde(rename = "browserExtension")]
    pub browser_extension: bool,
    /// Window management.
    #[serde(rename = "windowManagement")]
    pub window_management: bool,
    /// Setting the wallpaper.
    pub wallpaper: bool,
    /// File search.
    #[serde(rename = "fileSearch")]
    pub file_search: bool,
}

/// Everything the worker needs to start a command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadOptions {
    /// Whether the command draws a view.
    pub mode: CommandMode,
    /// Which worker build to use.
    pub env: CommandEnv,
    /// The data directory; the worker joins `support/` and `extensions/` to it.
    pub vicinae_path: String,
    /// The command's entry point, as a path.
    pub entrypoint: String,
    /// Whether this is a Raycast extension.
    pub is_raycast: bool,
    /// The command's id.
    pub command_name: String,
    /// The extension's id.
    pub extension_id: String,
    /// The extension's display name.
    pub extension_name: String,
    /// Who published it.
    pub owner_or_author_name: String,
    /// Argument values, shaped by the command's manifest.
    pub arguments: serde_json::Value,
    /// Preference values, likewise.
    pub preferences: serde_json::Value,
    /// Whatever launched it passed along.
    pub launch_context: serde_json::Value,
    /// What caused the launch.
    pub launch_type: LaunchType,
    /// What the extension may reach.
    pub capabilities: Capabilities,
    /// Fallback text, when the command was launched as a fallback.
    #[serde(
        rename = "fallbackText",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub fallback_text: Option<String>,
    /// The working directory, when one was chosen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

/// What `load` answers with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadResponse {
    /// Identifies this run of the command in every later call.
    pub session_id: String,
}

/// Something a worker raised on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// An extension sent the host a payload — itself an RPC message belonging
    /// to the extension protocol, which this layer does not interpret.
    ExtensionMessage {
        /// Which run.
        session_id: String,
        /// The opaque payload.
        payload: String,
    },
    /// An extension died, deliberately or otherwise.
    ExtensionCrash {
        /// Which run.
        session_id: String,
        /// What the worker said about it.
        reason: String,
    },
}

impl Event {
    /// Reads an event out of an incoming message, or `None` if it is a
    /// response or an event this host does not know.
    ///
    /// An unknown event is `None` rather than an error: the worker is free to
    /// gain events, and a host that refused to run against a newer worker
    /// because of one would be worse than a host that ignores it.
    #[must_use]
    pub fn from_incoming(message: &rpc::Incoming) -> Option<Self> {
        if !message.is_event() {
            return None;
        }
        let params = message.params.as_ref()?;
        let field = |name: &str| params.get(name)?.as_str().map(ToOwned::to_owned);

        match message.method.as_deref()? {
            rpc::manager::EVENT_EXTENSION_MESSAGE => Some(Self::ExtensionMessage {
                session_id: field("session_id")?,
                payload: field("payload")?,
            }),
            rpc::manager::EVENT_EXTENSION_CRASH => Some(Self::ExtensionCrash {
                session_id: field("session_id")?,
                reason: field("reason")?,
            }),
            _ => None,
        }
    }
}

/// Sends `Manager` calls to a [`Worker`].
///
/// Every method returns the request id rather than the reply, for the reason
/// [`Worker::request`] does: responses and events are interleaved on one pipe,
/// so reading is a separate concern from writing.
#[derive(Debug)]
pub struct ManagerClient<'a> {
    worker: &'a mut Worker,
}

impl<'a> ManagerClient<'a> {
    /// Wraps `worker`.
    pub fn new(worker: &'a mut Worker) -> Self {
        Self { worker }
    }

    /// Starts a command. The reply is a [`LoadResponse`].
    ///
    /// # Errors
    ///
    /// Whatever [`Worker::request`] can fail with.
    pub fn load(&mut self, opts: &LoadOptions) -> Result<u64, WorkerError> {
        self.worker
            .request(rpc::manager::LOAD, serde_json::json!({ "opts": opts }))
    }

    /// Tells the worker the host has stored the session id and is ready for
    /// that session's messages.
    ///
    /// `manager.fig` explains why this is a separate call: without it the host
    /// can miss an extension's first messages if the extension loads faster
    /// than the host stores the id.
    ///
    /// # Errors
    ///
    /// Whatever [`Worker::request`] can fail with.
    pub fn ready(&mut self, session_id: &str) -> Result<u64, WorkerError> {
        self.worker.request(
            rpc::manager::READY,
            serde_json::json!({ "session_id": session_id }),
        )
    }

    /// Stops a command.
    ///
    /// # Errors
    ///
    /// Whatever [`Worker::request`] can fail with.
    pub fn unload(&mut self, session_id: &str) -> Result<u64, WorkerError> {
        self.worker.request(
            rpc::manager::UNLOAD,
            serde_json::json!({ "session_id": session_id }),
        )
    }

    /// Forwards a payload to a running extension.
    ///
    /// # Errors
    ///
    /// Whatever [`Worker::request`] can fail with.
    pub fn message_extension(
        &mut self,
        session_id: &str,
        payload: &str,
    ) -> Result<u64, WorkerError> {
        self.worker.request(
            rpc::manager::MESSAGE_EXTENSION,
            serde_json::json!({ "session_id": session_id, "payload": payload }),
        )
    }
}

#[cfg(test)]
mod tests {
    //! These read `figura/manager.fig` and the TypeScript generator. A test
    //! that restated the field names would pass for ever while the IDL moved
    //! underneath it, which is the failure mode this whole crate exists to
    //! avoid.

    use super::*;
    use std::path::{Path, PathBuf};

    const FIG: &str = "figura/manager.fig";
    const WORKER_SERVER: &str = "src/typescript/extension-manager/src/proto/manager.ts";
    const WORKER_CLIENT: &str = "src/typescript/extension-manager/src/proto/api.ts";

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("crates/compass-worker-host sits two levels below the repository root")
            .to_path_buf()
    }

    fn read(rel: &str) -> String {
        let path = repo_root().join(rel);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    }

    /// The field or variant names declared inside `<keyword> <name> { … }`.
    ///
    /// Fields are `name: type;` and enum variants are bare names, so both come
    /// out of the same scan: take what precedes the first `:` or `,`, and drop
    /// the `?` an optional field carries.
    fn members_of(kind: &str, name: &str) -> Vec<String> {
        let fig = read(FIG);
        let opening = format!("{kind} {name} {{");
        let body = fig
            .split(&opening)
            .nth(1)
            .unwrap_or_else(|| panic!("{FIG} no longer declares `{opening}`"))
            .split('}')
            .next()
            .expect("the block is not closed");

        body.lines()
            .map(|line| line.split("//").next().unwrap_or("").trim())
            .filter(|line| !line.is_empty())
            .filter_map(|line| {
                let member = line.split([':', ',', ';']).next()?.trim();
                (!member.is_empty()).then(|| member.trim_end_matches('?').to_owned())
            })
            .collect()
    }

    /// The keys a serialised value carries, sorted.
    ///
    /// Sorted rather than in declaration order because `serde_json` stores an
    /// object in a `BTreeMap` unless its `preserve_order` feature is on, so key
    /// order here is an artefact of the map and not of the wire. JSON object
    /// order is not significant to the worker either, which reads by name.
    fn keys_of(value: &serde_json::Value) -> Vec<String> {
        let mut keys: Vec<String> = value
            .as_object()
            .expect("a struct serialises to an object")
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    }

    /// `names`, sorted, so a comparison against [`keys_of`] compares sets.
    fn sorted<S: AsRef<str>>(names: &[S]) -> Vec<String> {
        let mut out: Vec<String> = names.iter().map(|n| n.as_ref().to_owned()).collect();
        out.sort();
        out
    }

    fn a_load() -> LoadOptions {
        LoadOptions {
            mode: CommandMode::View,
            env: CommandEnv::Production,
            vicinae_path: "/home/u/.local/share/vicinae".to_owned(),
            entrypoint: "index.js".to_owned(),
            is_raycast: false,
            command_name: "search".to_owned(),
            extension_id: "hn".to_owned(),
            extension_name: "Hacker News".to_owned(),
            owner_or_author_name: "someone".to_owned(),
            arguments: serde_json::json!({}),
            preferences: serde_json::json!({}),
            launch_context: serde_json::Value::Null,
            launch_type: LaunchType::User,
            capabilities: Capabilities::default(),
            fallback_text: Some("hn ".to_owned()),
            cwd: Some("/tmp".to_owned()),
        }
    }

    #[test]
    fn the_load_options_field_names_match_the_idl() {
        let declared = members_of("struct", "LoadOptions");
        assert!(
            declared.len() >= 16,
            "parsed only {} fields from {FIG}; the parser has drifted from the IDL's shape \
             and is no longer checking anything: {declared:?}",
            declared.len()
        );

        let ours = keys_of(&serde_json::to_value(a_load()).expect("LoadOptions serialises"));
        for field in &declared {
            assert!(
                ours.contains(field),
                "{FIG} declares LoadOptions.{field}, which this struct does not send. The \
                 worker reads it as `undefined`, silently."
            );
        }
        assert_eq!(
            ours.len(),
            declared.len(),
            "this struct sends fields {FIG} does not declare: sent {ours:?}, declared {declared:?}"
        );
    }

    #[test]
    fn the_capability_field_names_match_the_idl() {
        let declared = members_of("struct", "Capabilities");
        assert_eq!(
            declared.len(),
            4,
            "parsed {declared:?} from {FIG}, which is not the shape this test expects"
        );

        let ours = keys_of(&serde_json::to_value(Capabilities::default()).expect("serialises"));
        assert_eq!(
            ours,
            sorted(&declared),
            "Capabilities' wire names have drifted from the IDL. A capability the worker does \
             not recognise is a capability the extension does not get."
        );
    }

    #[test]
    fn an_absent_optional_field_is_omitted_rather_than_null() {
        // The generated TypeScript passes `undefined` for an absent optional,
        // and `JSON.stringify` drops the key. A host that sent `null` instead
        // would make `if (load.cwd)` and `load.cwd ?? x` behave the same but
        // `"cwd" in load` differ, which is the kind of difference that shows up
        // only in the one code path that checks.
        let mut opts = a_load();
        opts.cwd = None;
        opts.fallback_text = None;

        let keys = keys_of(&serde_json::to_value(&opts).expect("serialises"));
        assert!(!keys.contains(&"cwd".to_owned()), "cwd should be absent");
        assert!(
            !keys.contains(&"fallbackText".to_owned()),
            "fallbackText should be absent"
        );
    }

    #[test]
    fn the_enum_variants_match_the_idl() {
        for (name, ours) in [
            (
                "CommandMode",
                vec![CommandMode::View, CommandMode::NoView]
                    .into_iter()
                    .map(|v| serde_json::to_value(v).expect("serialises"))
                    .collect::<Vec<_>>(),
            ),
            (
                "CommandEnv",
                vec![CommandEnv::Development, CommandEnv::Production]
                    .into_iter()
                    .map(|v| serde_json::to_value(v).expect("serialises"))
                    .collect(),
            ),
            (
                "LaunchType",
                vec![
                    LaunchType::User,
                    LaunchType::Background,
                    LaunchType::CommandLine,
                ]
                .into_iter()
                .map(|v| serde_json::to_value(v).expect("serialises"))
                .collect(),
            ),
        ] {
            let declared = members_of("enum", name);
            let spelled: Vec<String> = ours
                .iter()
                .map(|v| {
                    v.as_str()
                        .expect("an enum serialises to a string, as the worker compares it")
                        .to_owned()
                })
                .collect();
            assert_eq!(
                spelled, declared,
                "{name}'s variants disagree with {FIG}. The worker compares these as strings \
                 -- `env === \"Development\"`, `mode === \"NoView\"` -- so a misspelling takes \
                 the wrong branch instead of failing."
            );
        }
    }

    #[test]
    fn the_generator_still_keys_params_by_parameter_name() {
        // Everything below assumes it. If the generator switched to positional
        // params, or to passing a single struct unwrapped, every call in this
        // module would reach a handler with undefined arguments and no error.
        assert!(
            read(WORKER_CLIENT)
                .contains(r#"return this.transport.request("Storage/get", { key});"#),
            "{WORKER_CLIENT} no longer builds a named params object for requests"
        );
        assert!(
            read(WORKER_SERVER).contains("this.Manager.unload(msg.params.session_id)"),
            "{WORKER_SERVER} no longer routes server-side params by parameter name"
        );
    }

    #[test]
    fn each_call_sends_the_parameter_names_the_idl_declares() {
        // The params key for `fn load(opts: LoadOptions)` is `opts` -- the
        // parameter's name, not the struct's, and not the fields inlined.
        let fig = read(FIG);
        let service = fig
            .split("service Manager {")
            .nth(1)
            .expect("the Manager service")
            .split("\n}")
            .next()
            .expect("the service block is not closed");

        let mut checked = 0;
        for line in service.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix("fn ") else {
                continue;
            };
            let (name, tail) = rest.split_once('(').expect("a method has parameters");
            let params = tail.split(')').next().expect("the list is closed");
            let declared: Vec<&str> = params
                .split(',')
                .filter_map(|p| p.split(':').next())
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .collect();

            let sent = sent_params_for(name.trim());
            assert_eq!(
                sent,
                sorted(&declared),
                "Manager/{} sends {sent:?} but {FIG} declares ({declared:?})",
                name.trim()
            );
            checked += 1;
        }
        assert_eq!(
            checked, 4,
            "expected four methods on Manager, checked {checked}"
        );
    }

    /// The params keys each `ManagerClient` call actually puts on the wire.
    ///
    /// Read off a real call rather than written down: `cat` echoes the frame
    /// back, so what is asserted is what a worker would receive.
    fn sent_params_for(method: &str) -> Vec<String> {
        let mut worker =
            Worker::spawn(std::process::Command::new("cat")).expect("cat is spawnable");
        {
            let mut client = ManagerClient::new(&mut worker);
            match method {
                "load" => client.load(&a_load()),
                "unload" => client.unload("s"),
                "ready" => client.ready("s"),
                "messageExtension" => client.message_extension("s", "{}"),
                other => panic!("{FIG} declares Manager/{other}, which this test does not send"),
            }
            .expect("writing the request");
        }

        let message = worker
            .next_message()
            .expect("reading it back")
            .expect("cat echoed the frame");
        assert_eq!(
            message.method.as_deref(),
            Some(format!("Manager/{method}").as_str()),
            "the wire method name is `<Service>/<method>`"
        );
        keys_of(&message.params.expect("a request carries params"))
    }

    #[test]
    fn a_load_survives_the_round_trip_unchanged() {
        // The params wrapper has to unwrap to exactly what went in: a dropped
        // or renamed field inside `opts` would not be caught by the key check
        // above, which only looks at the wrapper.
        let opts = a_load();
        let mut worker =
            Worker::spawn(std::process::Command::new("cat")).expect("cat is spawnable");
        ManagerClient::new(&mut worker)
            .load(&opts)
            .expect("writing the request");

        let message = worker
            .next_message()
            .expect("reading it back")
            .expect("cat echoed the frame");
        let echoed: LoadOptions = serde_json::from_value(
            message
                .params
                .expect("params")
                .get("opts")
                .expect("load's params are keyed by `opts`")
                .clone(),
        )
        .expect("the echoed opts deserialise");

        assert_eq!(echoed, opts);
    }

    #[test]
    fn the_events_the_idl_declares_are_recognised() {
        let message = rpc::Incoming {
            jsonrpc: rpc::VERSION.to_owned(),
            id: None,
            method: Some(rpc::manager::EVENT_EXTENSION_MESSAGE.to_owned()),
            params: Some(serde_json::json!({ "session_id": "s", "payload": "{}" })),
            result: None,
        };
        assert_eq!(
            Event::from_incoming(&message),
            Some(Event::ExtensionMessage {
                session_id: "s".to_owned(),
                payload: "{}".to_owned(),
            })
        );

        let crash = rpc::Incoming {
            method: Some(rpc::manager::EVENT_EXTENSION_CRASH.to_owned()),
            params: Some(serde_json::json!({ "session_id": "s", "reason": "boom" })),
            ..message.clone()
        };
        assert_eq!(
            Event::from_incoming(&crash),
            Some(Event::ExtensionCrash {
                session_id: "s".to_owned(),
                reason: "boom".to_owned(),
            })
        );
    }

    #[test]
    fn the_event_parameter_names_match_the_idl() {
        // `extensionMessage(session_id, payload)` -- the generator emits
        // `{ session_id, payload }` and the handler reads `msg.session_id`.
        // Reading the wrong key here yields None, which presents as a lost
        // event rather than an error.
        let fig = read(FIG);
        for (event, sample) in [
            (
                "extensionMessage",
                serde_json::json!({ "session_id": "s", "payload": "{}" }),
            ),
            (
                "extensionCrash",
                serde_json::json!({ "session_id": "s", "reason": "boom" }),
            ),
        ] {
            let line = fig
                .lines()
                .find(|l| l.trim().starts_with(&format!("event {event}(")))
                .unwrap_or_else(|| panic!("{FIG} no longer declares `event {event}`"));
            let declared: Vec<&str> = line
                .split_once('(')
                .expect("parameters")
                .1
                .split(')')
                .next()
                .expect("closed")
                .split(',')
                .filter_map(|p| p.split(':').next())
                .map(str::trim)
                .collect();

            let keys = keys_of(&sample);
            assert_eq!(
                keys,
                sorted(&declared),
                "the sample for {event} does not carry the IDL's parameter names"
            );

            let message = rpc::Incoming {
                jsonrpc: rpc::VERSION.to_owned(),
                id: None,
                method: Some(format!("Manager/{event}")),
                params: Some(sample),
                result: None,
            };
            assert!(
                Event::from_incoming(&message).is_some(),
                "Manager/{event} with the IDL's own parameter names was not recognised"
            );
        }
    }

    #[test]
    fn a_response_is_not_an_event_and_an_unknown_event_is_ignored() {
        let response = rpc::Incoming {
            jsonrpc: rpc::VERSION.to_owned(),
            id: Some(1),
            method: None,
            params: None,
            result: Some(serde_json::json!({ "session_id": "s" })),
        };
        assert_eq!(Event::from_incoming(&response), None);

        let unknown = rpc::Incoming {
            jsonrpc: rpc::VERSION.to_owned(),
            id: None,
            method: Some("Manager/somethingNewer".to_owned()),
            params: Some(serde_json::json!({})),
            result: None,
        };
        assert_eq!(
            Event::from_incoming(&unknown),
            None,
            "an unknown event is ignored, not an error: the worker may gain events"
        );
    }
}
