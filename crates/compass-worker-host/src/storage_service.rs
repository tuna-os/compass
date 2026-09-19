//! The `Storage` half of the extension API.
//!
//! Ports `ExtStorageService` (`src/server/src/extension/api/storage-service.hpp`),
//! which is five methods over one [`compass_local_storage::Scoped`] namespace.
//! The store itself, and the several C++ behaviours it reproduces deliberately,
//! live in that crate; this is the part that turns a [`Call`] into a reply.
//!
//! # Void methods reply with `null`, not with nothing
//!
//! The glaze generator emits `m_transport.reply(req.id, nullptr)` for a method
//! returning `void`. The extension's client resolves its promise on `result`
//! either way, so not replying at all would hang the caller for ever rather
//! than failing it. `set`, `remove` and `clear` therefore answer `null`.

use compass_local_storage::{Error, Scoped, Value};

use crate::tsapi::{self, Call};

/// The methods this serves, as they appear on the wire.
pub const METHODS: &[&str] = &[
    "Storage/get",
    "Storage/set",
    "Storage/remove",
    "Storage/clear",
    "Storage/list",
];

/// Serves `Storage` for one extension's namespace.
#[derive(Debug)]
pub struct StorageService<'a> {
    scoped: Scoped<'a>,
}

impl<'a> StorageService<'a> {
    /// Serves `scoped`.
    #[must_use]
    pub fn new(scoped: Scoped<'a>) -> Self {
        Self { scoped }
    }

    /// Answers `call`, or `None` if it is not a `Storage` call.
    ///
    /// The answer is a payload ready for `Manager/messageExtension`. An event
    /// — a call with no id — is also `None`: `Storage` declares none, and
    /// replying to one would leave a reply the client drops.
    #[must_use]
    pub fn handle(&self, call: &Call) -> Option<String> {
        let id = call.id?;
        if !METHODS.contains(&call.method.as_str()) {
            return None;
        }

        Some(match self.answer(call) {
            Ok(result) => tsapi::reply(id, result),
            Err(error) => tsapi::reply_error(id, &error.to_string()),
        })
    }

    /// The result value for a call already known to be ours.
    fn answer(&self, call: &Call) -> Result<serde_json::Value, Error> {
        let key = || {
            call.params
                .get("key")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };

        match call.method.as_str() {
            "Storage/get" => Ok(self
                .scoped
                .get(&key())?
                .map_or(serde_json::Value::Null, |value| value.to_json())),
            "Storage/set" => {
                let value = Value::from_json(call.params.get("value").unwrap_or(&NULL));
                self.scoped.set(&key(), &value)?;
                Ok(serde_json::Value::Null)
            }
            "Storage/remove" => {
                self.scoped.remove(&key())?;
                Ok(serde_json::Value::Null)
            }
            "Storage/clear" => {
                self.scoped.clear()?;
                Ok(serde_json::Value::Null)
            }
            "Storage/list" => {
                let mut out = serde_json::Map::new();
                for (key, value) in self.scoped.list()? {
                    out.insert(key, value.to_json());
                }
                Ok(serde_json::Value::Object(out))
            }
            // Unreachable: `handle` checked the name. Answering an error rather
            // than panicking, because a panic here would take the host down
            // over an extension's message.
            other => Ok(serde_json::Value::String(format!(
                "{other} is not a Storage method"
            ))),
        }
    }
}

/// A `null` to borrow when `params.value` is absent.
const NULL: serde_json::Value = serde_json::Value::Null;

impl tsapi::Service for StorageService<'_> {
    fn handle(&self, call: &Call) -> Option<String> {
        Self::handle(self, call)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_local_storage::{LocalStorage, namespace_for, schema};
    use compass_sqlcipher_sys::Database;

    fn call(method: &str, params: serde_json::Value) -> Call {
        Call {
            jsonrpc: crate::rpc::VERSION.to_owned(),
            id: Some(1),
            method: method.to_owned(),
            params,
        }
    }

    fn result_of(service: &StorageService<'_>, call: &Call) -> serde_json::Value {
        let payload = service.handle(call).expect("a Storage call is answered");
        let answer: serde_json::Value = serde_json::from_str(&payload).expect("a JSON reply");
        assert_eq!(answer["id"], 1);
        assert!(
            answer.get("error").is_none(),
            "unexpected error: {}",
            answer["error"]
        );
        answer["result"].clone()
    }

    fn open() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let db = Database::open(&dir.path().join("vicinae.db"), &[]).expect("an unencrypted db");
        schema::run(&db).expect("the migrations apply");
        (dir, db)
    }

    #[test]
    fn the_methods_are_the_ones_the_idl_declares() {
        // The ledger in `tsapi` is checked against the IDL; this list is what
        // puts entries on it, so the two must not drift.
        for method in METHODS {
            assert!(
                tsapi::is_implemented(method),
                "{method} is served here but is not on the tsapi ledger"
            );
        }
    }

    #[test]
    fn a_value_set_by_one_call_is_read_by_the_next() {
        let (_dir, db) = open();
        let storage = LocalStorage::new(&db);
        let service = StorageService::new(storage.scoped(&namespace_for("hn")));

        assert_eq!(
            result_of(
                &service,
                &call(
                    "Storage/set",
                    serde_json::json!({ "key": "k", "value": "v" })
                )
            ),
            serde_json::Value::Null,
            "a void method answers null; answering nothing hangs the caller"
        );
        assert_eq!(
            result_of(
                &service,
                &call("Storage/get", serde_json::json!({ "key": "k" }))
            ),
            serde_json::json!("v")
        );
    }

    #[test]
    fn a_missing_key_reads_as_null() {
        let (_dir, db) = open();
        let storage = LocalStorage::new(&db);
        let service = StorageService::new(storage.scoped("x:data"));

        assert_eq!(
            result_of(
                &service,
                &call("Storage/get", serde_json::json!({ "key": "nope" }))
            ),
            serde_json::Value::Null
        );
    }

    #[test]
    fn list_answers_an_object_keyed_by_key() {
        let (_dir, db) = open();
        let storage = LocalStorage::new(&db);
        let service = StorageService::new(storage.scoped("x:data"));

        service
            .handle(&call(
                "Storage/set",
                serde_json::json!({ "key": "a", "value": 1 }),
            ))
            .expect("set");
        service
            .handle(&call(
                "Storage/set",
                serde_json::json!({ "key": "b", "value": true }),
            ))
            .expect("set");

        assert_eq!(
            result_of(&service, &call("Storage/list", serde_json::json!({}))),
            serde_json::json!({ "a": 1.0, "b": true }),
            "`list` is a `QJsonObject` on the C++ side, not an array of pairs"
        );
    }

    #[test]
    fn remove_and_clear_take_effect() {
        let (_dir, db) = open();
        let storage = LocalStorage::new(&db);
        let service = StorageService::new(storage.scoped("x:data"));

        for key in ["a", "b"] {
            service
                .handle(&call(
                    "Storage/set",
                    serde_json::json!({ "key": key, "value": "v" }),
                ))
                .expect("set");
        }

        result_of(
            &service,
            &call("Storage/remove", serde_json::json!({ "key": "a" })),
        );
        assert_eq!(
            result_of(&service, &call("Storage/list", serde_json::json!({}))),
            serde_json::json!({ "b": "v" })
        );

        result_of(&service, &call("Storage/clear", serde_json::json!({})));
        assert_eq!(
            result_of(&service, &call("Storage/list", serde_json::json!({}))),
            serde_json::json!({})
        );
    }

    #[test]
    fn a_call_for_another_service_is_left_alone() {
        // The host will have more than one service; each must decline what is
        // not its own rather than answering an error, or the first one asked
        // would refuse every call in the system.
        let (_dir, db) = open();
        let storage = LocalStorage::new(&db);
        let service = StorageService::new(storage.scoped("x:data"));

        assert_eq!(
            service.handle(&call("UI/render", serde_json::json!({ "json": "{}" }))),
            None
        );
    }

    #[test]
    fn an_event_is_not_answered() {
        let (_dir, db) = open();
        let storage = LocalStorage::new(&db);
        let service = StorageService::new(storage.scoped("x:data"));

        let mut event = call("Storage/clear", serde_json::json!({}));
        event.id = None;
        assert_eq!(
            service.handle(&event),
            None,
            "a reply to an event carries an id no client waits on"
        );
    }

    #[test]
    fn a_store_error_is_an_error_reply_rather_than_a_panic() {
        // A row this build does not understand reaches the extension as a
        // rejected promise naming the problem, not as a dead host.
        let (_dir, db) = open();
        db.execute(
            "INSERT INTO storage_data_item (namespace_id, value_type, key, value) \
             VALUES ('x:data', 9, 'k', 'v')",
        )
        .expect("a row from a newer build");

        let storage = LocalStorage::new(&db);
        let service = StorageService::new(storage.scoped("x:data"));

        let payload = service
            .handle(&call("Storage/get", serde_json::json!({ "key": "k" })))
            .expect("answered");
        let answer: serde_json::Value = serde_json::from_str(&payload).expect("JSON");
        let message = answer["error"].as_str().expect("a string error");
        assert!(
            message.contains("value_type 9"),
            "the refusal must say what it could not read: {message}"
        );
    }
}
