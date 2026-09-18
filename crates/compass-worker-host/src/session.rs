//! One running extension: the loop between a worker and the host's services.
//!
//! The shape is set by the two protocols this crate already knows. The worker
//! raises `Manager/extensionMessage` events carrying an opaque payload; that
//! payload is a [`crate::tsapi`] call; the answer goes back as
//! `Manager/messageExtension` on the same session id. [`Router`] is the pure
//! half — a call in, a payload or nothing out — and [`Session`] is the half
//! that touches a process.
//!
//! # A call for a session that is not ours is not ours to answer
//!
//! One worker process serves many sessions, so every event carries the id and
//! every reply has to quote it back. A host that answered on the wrong session
//! would deliver an extension's storage to a different extension.

use crate::extension_manager::{Event, ManagerClient};
use crate::tsapi::{self, Call, Service};
use crate::{Worker, WorkerError};

/// The services a host offers, asked in order.
#[derive(Default)]
pub struct Router<'a> {
    services: Vec<&'a dyn Service>,
}

impl std::fmt::Debug for Router<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Router")
            .field("services", &self.services.len())
            .finish()
    }
}

impl<'a> Router<'a> {
    /// A router with no services, which answers every call with
    /// [`tsapi::unimplemented`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a service.
    #[must_use]
    pub fn with(mut self, service: &'a dyn Service) -> Self {
        self.services.push(service);
        self
    }

    /// The answer to `payload`, or `None` if none is owed.
    ///
    /// `None` means one of three things, and none of them is an error: the
    /// payload was an event, or it was not a call at all. A payload that does
    /// not parse is not answered because there is no id to answer *to* — a
    /// reply with a guessed id would resolve a promise the extension is
    /// waiting on for something else.
    #[must_use]
    pub fn route(&self, payload: &str) -> Option<String> {
        let call = tsapi::parse(payload).ok()?;
        self.route_call(&call)
    }

    /// As [`route`](Self::route), for a call that is already parsed.
    #[must_use]
    pub fn route_call(&self, call: &Call) -> Option<String> {
        if call.is_event() {
            return None;
        }
        for service in &self.services {
            if let Some(answer) = service.handle(call) {
                return Some(answer);
            }
        }
        // Every call carries an id, and an unanswered one is an extension
        // waiting for ever.
        call.id.map(|id| tsapi::unimplemented(id, &call.method))
    }
}

/// What one turn of the loop did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Turn {
    /// A call was answered and the reply sent.
    Answered {
        /// Which method.
        method: String,
    },
    /// A message arrived that needed no answer — an event, or a response to
    /// something the host asked.
    Nothing,
    /// The extension crashed. The worker says why.
    Crashed {
        /// What the worker reported.
        reason: String,
    },
    /// A message for another session, left alone.
    OtherSession {
        /// Whose it was.
        session_id: String,
    },
    /// The worker's output ended.
    Closed,
}

/// One extension session over one worker.
#[derive(Debug)]
pub struct Session<'a> {
    worker: Worker,
    session_id: String,
    router: Router<'a>,
}

impl<'a> Session<'a> {
    /// Binds `worker`'s `session_id` to `router`.
    #[must_use]
    pub fn new(worker: Worker, session_id: impl Into<String>, router: Router<'a>) -> Self {
        Self {
            worker,
            session_id: session_id.into(),
            router,
        }
    }

    /// The session id this answers on.
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// The worker, for the calls a session does not make itself.
    pub fn worker_mut(&mut self) -> &mut Worker {
        &mut self.worker
    }

    /// Reads one message and does whatever it asks for.
    ///
    /// # Errors
    ///
    /// [`WorkerError`] if the worker's stream fails, ends mid-frame, or sends
    /// something that is not JSON-RPC.
    pub fn pump_once(&mut self) -> Result<Turn, WorkerError> {
        let Some(message) = self.worker.next_message()? else {
            return Ok(Turn::Closed);
        };

        let Some(event) = Event::from_incoming(&message) else {
            return Ok(Turn::Nothing);
        };

        match event {
            Event::ExtensionCrash { session_id, reason } => {
                if session_id != self.session_id {
                    return Ok(Turn::OtherSession { session_id });
                }
                Ok(Turn::Crashed { reason })
            }
            Event::ExtensionMessage {
                session_id,
                payload,
            } => {
                if session_id != self.session_id {
                    return Ok(Turn::OtherSession { session_id });
                }
                let Some(reply) = self.router.route(&payload) else {
                    return Ok(Turn::Nothing);
                };
                let method = tsapi::parse(&payload)
                    .map(|call| call.method)
                    .unwrap_or_default();

                ManagerClient::new(&mut self.worker).message_extension(&self.session_id, &reply)?;
                Ok(Turn::Answered { method })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode;
    use crate::rpc;
    use compass_local_storage::{LocalStorage, namespace_for};
    use compass_sqlcipher_sys::Database;
    use std::path::PathBuf;

    /// A service that answers one method with a fixed value, so routing can be
    /// tested without a database.
    struct Fixed {
        method: &'static str,
        result: &'static str,
    }

    impl Service for Fixed {
        fn handle(&self, call: &Call) -> Option<String> {
            let id = call.id?;
            (call.method == self.method).then(|| tsapi::reply(id, serde_json::json!(self.result)))
        }
    }

    fn call(method: &str, id: Option<u64>) -> String {
        let mut value = serde_json::json!({
            "jsonrpc": rpc::VERSION,
            "method": method,
            "params": {},
        });
        if let Some(id) = id {
            value["id"] = serde_json::json!(id);
        }
        value.to_string()
    }

    #[test]
    fn the_first_service_that_claims_a_call_answers_it() {
        let first = Fixed {
            method: "UI/render",
            result: "first",
        };
        let second = Fixed {
            method: "UI/render",
            result: "second",
        };
        let router = Router::new().with(&first).with(&second);

        let reply = router.route(&call("UI/render", Some(1))).expect("answered");
        let value: serde_json::Value = serde_json::from_str(&reply).expect("JSON");
        assert_eq!(value["result"], "first", "services are asked in order");
        assert_eq!(value["id"], 1);
    }

    #[test]
    fn a_call_nobody_claims_is_refused_by_name_rather_than_ignored() {
        // An unanswered call is an extension waiting for ever. The refusal
        // names the method because the extension's stack trace stops at the
        // generated client.
        let router = Router::new();
        let reply = router
            .route(&call("Wallpaper/set", Some(7)))
            .expect("refused");
        let value: serde_json::Value = serde_json::from_str(&reply).expect("JSON");
        assert_eq!(value["id"], 7);
        assert!(
            value["error"]
                .as_str()
                .is_some_and(|e| e.contains("Wallpaper/set")),
            "the refusal must name the method: {value}"
        );
    }

    #[test]
    fn an_event_and_an_unparseable_payload_are_both_left_alone() {
        let router = Router::new();
        assert_eq!(
            router.route(&call("UI/viewPoped", None)),
            None,
            "an event expects no answer"
        );
        assert_eq!(
            router.route("not json at all"),
            None,
            "there is no id to answer to, and a guessed one would resolve someone else's promise"
        );
    }

    /// A child that prints `bytes` and then copies its stdin to `sink`.
    fn replay_then_capture(
        bytes: &[u8],
        dir: &std::path::Path,
    ) -> (PathBuf, std::process::Command) {
        let frames = dir.join("frames.bin");
        let sink = dir.join("captured.bin");
        std::fs::write(&frames, bytes).expect("staging the worker's output");

        let mut command = std::process::Command::new("sh");
        command.arg("-c").arg(format!(
            "cat {}; cat > {}",
            frames.to_string_lossy(),
            sink.to_string_lossy()
        ));
        (sink, command)
    }

    fn event(session_id: &str, payload: &str) -> Vec<u8> {
        let message = serde_json::json!({
            "jsonrpc": rpc::VERSION,
            "method": rpc::manager::EVENT_EXTENSION_MESSAGE,
            "params": { "session_id": session_id, "payload": payload },
        });
        encode(&serde_json::to_vec(&message).expect("serialises")).expect("a small frame")
    }

    fn open_db(dir: &std::path::Path) -> Database {
        let db = Database::open(&dir.join("vicinae.db"), &[]).expect("an unencrypted db");
        compass_db::vicinae::run(&db).expect("the migrations apply");
        db
    }

    #[test]
    fn a_storage_call_from_a_worker_is_answered_back_to_the_worker() {
        // The whole loop, over a real process: an extensionMessage event
        // carrying a Storage/set call goes in, and what comes out of the host
        // is a messageExtension request on the same session carrying the
        // reply.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let db = open_db(dir.path());
        let storage = LocalStorage::new(&db);
        let service =
            crate::storage_service::StorageService::new(storage.scoped(&namespace_for("hn")));

        let payload = serde_json::json!({
            "jsonrpc": rpc::VERSION,
            "id": 11,
            "method": "Storage/set",
            "params": { "key": "k", "value": "v" },
        })
        .to_string();

        let (sink, command) = replay_then_capture(&event("s-1", &payload), dir.path());
        let worker = Worker::spawn(command).expect("sh is spawnable");
        let mut session = Session::new(worker, "s-1", Router::new().with(&service));

        assert_eq!(
            session.pump_once().expect("one turn"),
            Turn::Answered {
                method: "Storage/set".to_owned()
            }
        );

        // Closing stdin lets the child's second `cat` finish writing.
        session.worker.shutdown().expect("the worker exits");

        let captured = std::fs::read(&sink).expect("the child captured our writes");
        let frame = crate::decode(&captured)
            .expect("a well-formed frame")
            .expect("there is one");
        let sent: serde_json::Value =
            serde_json::from_slice(frame.payload).expect("the host wrote JSON-RPC");

        assert_eq!(sent["method"], rpc::manager::MESSAGE_EXTENSION);
        assert_eq!(
            sent["params"]["session_id"], "s-1",
            "the reply must quote the session it answers"
        );
        let inner: serde_json::Value = serde_json::from_str(
            sent["params"]["payload"]
                .as_str()
                .expect("a string payload"),
        )
        .expect("the inner payload is JSON");
        assert_eq!(inner["id"], 11, "the reply must quote the call's id");
        assert!(inner.get("error").is_none(), "unexpected error: {inner}");

        // And the write actually happened.
        assert!(
            storage
                .scoped(&namespace_for("hn"))
                .get("k")
                .expect("read")
                .is_some()
        );
    }

    #[test]
    fn a_message_for_another_session_is_left_alone() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let payload = serde_json::json!({
            "jsonrpc": rpc::VERSION,
            "id": 1,
            "method": "Storage/clear",
            "params": {},
        })
        .to_string();

        let (sink, command) = replay_then_capture(&event("someone-else", &payload), dir.path());
        let worker = Worker::spawn(command).expect("sh is spawnable");
        let mut session = Session::new(worker, "s-1", Router::new());

        assert_eq!(
            session.pump_once().expect("one turn"),
            Turn::OtherSession {
                session_id: "someone-else".to_owned()
            },
            "answering another session would deliver one extension's data to another"
        );

        session.worker.shutdown().expect("the worker exits");
        assert!(
            std::fs::read(&sink).expect("the sink exists").is_empty(),
            "the host answered a session that is not its own"
        );
    }

    #[test]
    fn a_crash_is_reported_rather_than_answered() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let crash = serde_json::json!({
            "jsonrpc": rpc::VERSION,
            "method": rpc::manager::EVENT_EXTENSION_CRASH,
            "params": { "session_id": "s-1", "reason": "boom" },
        });
        let bytes =
            encode(&serde_json::to_vec(&crash).expect("serialises")).expect("a small frame");

        let (_sink, command) = replay_then_capture(&bytes, dir.path());
        let worker = Worker::spawn(command).expect("sh is spawnable");
        let mut session = Session::new(worker, "s-1", Router::new());

        assert_eq!(
            session.pump_once().expect("one turn"),
            Turn::Crashed {
                reason: "boom".to_owned()
            }
        );
    }

    #[test]
    fn the_end_of_the_workers_output_is_a_turn_not_an_error() {
        // A child that exits rather than one that waits on stdin: the
        // capturing child used above never closes its own output, and there is
        // nothing to capture here anyway.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let frames = dir.path().join("empty.bin");
        std::fs::write(&frames, b"").expect("an empty file");
        let mut command = std::process::Command::new("cat");
        command.arg(&frames);
        let worker = Worker::spawn(command).expect("cat is spawnable");
        let mut session = Session::new(worker, "s-1", Router::new());

        assert_eq!(session.pump_once().expect("one turn"), Turn::Closed);
    }
}
