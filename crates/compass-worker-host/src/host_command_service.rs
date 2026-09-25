//! The `HostCommand` half of the extension API: a program the engine runs on
//! the host for an extension, outside the extension's sandbox.
//!
//! An extension written for macOS shells out to tools it expects to find:
//! Raycast's Brew extension runs `/opt/homebrew/bin/brew`. On Linux the tool
//! may well be there (Linuxbrew, under `/home/linuxbrew/.linuxbrew`), but the
//! extension's Landlock policy does not let it execute anything there, and
//! inside the Flatpak the directory is not even visible. Rather than widen
//! the policy for every extension, the runtime's shim forwards the few
//! programs the engine brokers to this call, and the engine runs them after
//! asking the person (`crates/vicinae/src/host_commands.rs`).
//!
//! So `HostCommand/run` always defers: whether it runs at all is a person's
//! answer. What arrives is checked here first, and a request that could never
//! run is refused at once rather than shown to anyone: a `program` that is a
//! path, or not a plain name, would let the extension pick the binary the
//! person is agreeing to.

use crate::tsapi::{self, Call};

/// Every method this serves answers later, through a [`Broker`].
pub const METHODS: &[&str] = &[];

/// The method that waits on the person's answer.
pub const DEFERRED_METHODS: &[&str] = &["HostCommand/run"];

/// `HostCommandRequest`, as the runtime's shim sends it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Request {
    /// The program's bare name, `brew`: the engine resolves it.
    pub program: String,
    /// Its arguments, passed as they are, never through a shell.
    pub args: Vec<String>,
    /// What to write to its standard input.
    pub input: Option<String>,
    /// Environment variables the extension set, in its order. The engine
    /// keeps only the ones [`is_forwarded_env`] allows.
    pub env: Vec<(String, String)>,
}

impl Request {
    /// Reads `params.request`. `None` when it is not a request at all.
    #[must_use]
    pub fn from_call(call: &Call) -> Option<Self> {
        let request = call.params.get("request")?;
        let program = request.get("program")?.as_str()?.to_owned();
        let args = request
            .get("args")
            .and_then(serde_json::Value::as_array)
            .map(|args| {
                args.iter()
                    .filter_map(|arg| arg.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let input = request
            .get("input")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned);
        let env = request
            .get("env")
            .and_then(serde_json::Value::as_array)
            .map(|env| {
                env.iter()
                    .filter_map(|pair| {
                        Some((
                            pair.get("name")?.as_str()?.to_owned(),
                            pair.get("value")?.as_str()?.to_owned(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            program,
            args,
            input,
            env,
        })
    }
}

/// `HostCommandResult`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Outcome {
    /// The exit status; `-1` when a signal ended it.
    pub exit_code: i32,
    /// Its standard output, lossily decoded.
    pub stdout: String,
    /// Its standard error, lossily decoded.
    pub stderr: String,
}

impl Outcome {
    /// The reply's `result`, in the IDL's camelCase.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "exitCode": self.exit_code,
            "stdout": self.stdout,
            "stderr": self.stderr,
        })
    }
}

/// Whether `program` is a plain program name: letters, digits and `._+-`,
/// not starting with `-` or `.`. A path, an option or anything a shell or
/// `env` would read as more than a name is not.
#[must_use]
pub fn is_program_name(program: &str) -> bool {
    !program.is_empty()
        && program.len() <= 64
        && !program.starts_with(['-', '.'])
        && program
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-'))
}

/// The Homebrew switches an extension may pass through the environment.
const FORWARDED_ENV: &[&str] = &[
    "HOMEBREW_COLOR",
    "HOMEBREW_DOWNLOAD_CONCURRENCY",
    "HOMEBREW_VERBOSE",
];

/// Whether an environment variable the extension set reaches the host
/// program.
///
/// Only Homebrew's on/off switches (`HOMEBREW_NO_AUTO_UPDATE` and the other
/// `HOMEBREW_NO_*`) and a few like them. Never a variable naming a program or
/// a path — `HOMEBREW_BROWSER`, `SUDO_ASKPASS`, `LD_PRELOAD`, `PATH` — since
/// the host program runs unconfined and any of those would run something the
/// person did not agree to.
#[must_use]
pub fn is_forwarded_env(name: &str) -> bool {
    FORWARDED_ENV.contains(&name)
        || name.strip_prefix("HOMEBREW_NO_").is_some_and(|rest| {
            !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_uppercase() || b == b'_')
        })
}

/// Where a `HostCommand/run` call goes once it is known to be well formed.
pub trait Broker {
    /// Takes `request` and must settle `deferral`: with an [`Outcome`] once
    /// the program ran, or with a sentence when it may not or could not.
    fn request(&self, request: Request, deferral: &tsapi::Deferral);
}

/// Serves `HostCommand/run` through a [`Broker`].
#[derive(Debug)]
pub struct HostCommandService<B> {
    broker: B,
}

impl<B: Broker> HostCommandService<B> {
    /// Serves calls through `broker`.
    pub const fn new(broker: B) -> Self {
        Self { broker }
    }

    /// The broker.
    pub const fn broker(&self) -> &B {
        &self.broker
    }
}

/// Why `request` can never run, or `None` when it may be asked about.
#[must_use]
pub fn refusal(request: &Request) -> Option<String> {
    if !is_program_name(&request.program) {
        return Some(format!(
            "HostCommand/run takes a program's name, not {:?}: Compass decides which binary runs",
            request.program
        ));
    }
    None
}

impl<B: Broker> tsapi::Service for HostCommandService<B> {
    fn handle(&self, call: &Call) -> Option<String> {
        if !DEFERRED_METHODS.contains(&call.method.as_str()) {
            return None;
        }
        let id = call.id?;
        match Request::from_call(call) {
            None => Some(tsapi::reply_error(id, "HostCommand/run needs a request")),
            Some(request) => refusal(&request).map(|why| tsapi::reply_error(id, &why)),
        }
    }

    fn defer(&self, call: &Call) -> Option<tsapi::Deferral> {
        if !DEFERRED_METHODS.contains(&call.method.as_str()) {
            return None;
        }
        let request = Request::from_call(call)?;
        if refusal(&request).is_some() {
            return None;
        }
        let deferral = tsapi::Deferral::for_call(call)?;
        self.broker.request(request, &deferral);
        Some(deferral)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsapi::Service as _;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorded(Mutex<Vec<(Request, u64)>>);

    impl Broker for &Recorded {
        fn request(&self, request: Request, deferral: &tsapi::Deferral) {
            self.0.lock().unwrap().push((request, deferral.id));
        }
    }

    fn call(params: serde_json::Value) -> Call {
        serde_json::from_value(serde_json::json!({
            "jsonrpc": "2.0", "id": 4, "method": "HostCommand/run", "params": params,
        }))
        .unwrap()
    }

    #[test]
    fn a_request_defers_with_what_the_generated_client_sends() {
        let recorded = Recorded::default();
        let service = HostCommandService::new(&recorded);
        let call = call(serde_json::json!({ "request": {
            "program": "brew",
            "args": ["info", "--json=v2", "--installed"],
            "env": [{ "name": "HOMEBREW_NO_AUTO_UPDATE", "value": "1" }],
        }}));
        assert_eq!(
            service.handle(&call),
            None,
            "a good request is not answered now"
        );
        let deferral = service.defer(&call).expect("deferred");
        assert_eq!(deferral.id, 4);
        let recorded = recorded.0.lock().unwrap();
        assert_eq!(
            recorded[0].0,
            Request {
                program: "brew".into(),
                args: vec!["info".into(), "--json=v2".into(), "--installed".into()],
                input: None,
                env: vec![("HOMEBREW_NO_AUTO_UPDATE".into(), "1".into())],
            }
        );
    }

    #[test]
    fn a_path_or_an_option_is_refused_before_anyone_is_asked() {
        let recorded = Recorded::default();
        let service = HostCommandService::new(&recorded);
        for program in [
            "/opt/homebrew/bin/brew",
            "../brew",
            "-i",
            "",
            "a=b",
            "br ew",
            ".brew",
        ] {
            let call = call(
                serde_json::json!({ "request": { "program": program, "args": [], "env": [] }}),
            );
            let reply = service.handle(&call).expect("refused at once");
            assert!(reply.contains("error"), "{program}: {reply}");
            assert_eq!(service.defer(&call), None, "{program}");
        }
        assert!(recorded.0.lock().unwrap().is_empty());
    }

    #[test]
    fn only_homebrew_switches_cross_to_the_host() {
        for name in [
            "HOMEBREW_NO_AUTO_UPDATE",
            "HOMEBREW_NO_ENV_HINTS",
            "HOMEBREW_DOWNLOAD_CONCURRENCY",
        ] {
            assert!(is_forwarded_env(name), "{name}");
        }
        for name in [
            "PATH",
            "LD_PRELOAD",
            "SUDO_ASKPASS",
            "HOMEBREW_BROWSER",
            "HOMEBREW_EDITOR",
            "HOMEBREW_NO_",
            "HOMEBREW_NO_x",
            "HOME",
        ] {
            assert!(!is_forwarded_env(name), "{name}");
        }
    }

    #[test]
    fn the_result_uses_the_idl_names() {
        let outcome = Outcome {
            exit_code: 1,
            stdout: "a".into(),
            stderr: "b".into(),
        };
        assert_eq!(
            outcome.to_json(),
            serde_json::json!({ "exitCode": 1, "stdout": "a", "stderr": "b" })
        );
    }
}
