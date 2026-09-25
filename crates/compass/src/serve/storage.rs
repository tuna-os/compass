//! Inspect Local Storage and Manage OAuth Token Sets (IPC v19): the Vicinae
//! extension's two views over the encrypted database extensions keep their
//! data in, as `LocalStorageViewHost` and `OAuthTokenStoreViewHost` read the
//! C++'s `LocalStorageService` and `OAuth::TokenStore`.
//!
//! The database is the one calculator history opens, keyed from the login
//! keyring; without a keyring both are refused by name.

use std::sync::Arc;

use compass_ipc::{
    ErrorKind, LocalStorageEntry, OAuthTokenSetEntry, ProtocolError, Request, Response,
};
use compass_local_storage::LocalStorage;
use compass_oauth_store::{TokenSet, TokenStore};
use compass_sqlcipher_sys::rusqlite::Connection;
use tokio::sync::RwLock;

use super::EngineState;

const NO_KEYRING: &str = "the extension storage needs the login keyring to open its database, \
                          and there is none on this session";

/// A value as the view shows it (`QVariant::toString`): a string as it is, a
/// whole number without its `.0` (local storage keeps numbers as doubles),
/// anything else as its JSON.
fn value_text(value: &compass_local_storage::Value) -> String {
    match value.to_json() {
        serde_json::Value::String(text) => text,
        serde_json::Value::Number(number) => match number.as_f64() {
            #[allow(clippy::cast_possible_truncation)]
            Some(whole) if whole.fract() == 0.0 && whole.abs() < 1e15 => (whole as i64).to_string(),
            _ => number.to_string(),
        },
        other => other.to_string(),
    }
}

/// A token set on the wire; expired when its lifetime has run out by `now`,
/// as `TokenSet::isExpired`.
fn token_entry(set: TokenSet, now: i64) -> OAuthTokenSetEntry {
    let expires_at = set
        .expires_in
        .map(|seconds| set.updated_at.saturating_add(seconds));
    OAuthTokenSetEntry {
        expired: expires_at.is_some_and(|at| at <= now),
        expires_at,
        extension_id: set.extension_id,
        provider_id: set.provider_id,
        access_token: set.access_token,
        refresh_token: set.refresh_token,
        id_token: set.id_token,
        scope: set.scope,
    }
}

/// Answers a storage request against an open database, at `now` (seconds).
pub fn answer(db: &Connection, request: Request, now: i64) -> Response {
    let internal = |error: &dyn std::fmt::Display| {
        Response::Error(ProtocolError::new(ErrorKind::Internal, error.to_string()))
    };
    match request {
        Request::LocalStorageNamespaces => match LocalStorage::new(db).namespaces() {
            Ok(mut namespaces) => {
                namespaces.sort();
                Response::LocalStorageNamespaces { namespaces }
            }
            Err(error) => internal(&error),
        },
        Request::LocalStorageItems { namespace } => {
            let storage = LocalStorage::new(db);
            match storage.scoped(&namespace).list() {
                Ok(mut items) => {
                    items.sort_by(|a, b| a.0.cmp(&b.0));
                    Response::LocalStorageItems {
                        items: items
                            .into_iter()
                            .map(|(key, value)| LocalStorageEntry {
                                value: value_text(&value),
                                key,
                            })
                            .collect(),
                    }
                }
                Err(error) => internal(&error),
            }
        }
        Request::OAuthTokenSets => match TokenStore::new(db).list() {
            Ok(sets) => Response::OAuthTokenSets {
                sets: sets.into_iter().map(|set| token_entry(set, now)).collect(),
            },
            Err(error) => internal(&error),
        },
        Request::RemoveOAuthTokenSet {
            extension_id,
            provider_id,
        } => match TokenStore::new(db).remove(&extension_id, provider_id.as_deref()) {
            Ok(()) => Response::Ack,
            Err(_) => Response::Error(ProtocolError::new(
                ErrorKind::Internal,
                "Failed to remove token set",
            )),
        },
        other => Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            format!("not a storage request: {other:?}"),
        )),
    }
}

/// A storage request.
pub async fn handle(state: &Arc<RwLock<EngineState>>, request: Request) -> Response {
    let Some(storage) = super::calculator::storage(state).await else {
        return Response::Error(ProtocolError::new(ErrorKind::Unsupported, NO_KEYRING));
    };
    tokio::task::spawn_blocking(move || {
        let Some(db) = crate::extension_runner::open_storage(&storage) else {
            return Response::Error(ProtocolError::new(
                ErrorKind::Internal,
                "the extension storage's database would not open",
            ));
        };
        answer(&db, request, jiff::Timestamp::now().as_second())
    })
    .await
    .unwrap_or_else(|error| {
        Response::Error(ProtocolError::new(
            ErrorKind::Internal,
            format!("storage task failed: {error}"),
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db = compass_sqlcipher_sys::open(&dir.path().join("vicinae.db"), &[]).unwrap();
        compass_db::vicinae::run(&db).unwrap();
        (dir, db)
    }

    #[test]
    fn local_storage_lists_its_namespaces_and_their_items_as_text() {
        let (_dir, db) = open();
        let storage = LocalStorage::new(&db);
        let notes = storage.scoped("@zoë/notes");
        notes
            .set(
                "draft",
                &compass_local_storage::Value::from_json(&serde_json::json!("hello")),
            )
            .unwrap();
        notes
            .set(
                "count",
                &compass_local_storage::Value::from_json(&serde_json::json!(3)),
            )
            .unwrap();
        storage
            .scoped("core")
            .set(
                "seen",
                &compass_local_storage::Value::from_json(&serde_json::json!(true)),
            )
            .unwrap();

        assert_eq!(
            answer(&db, Request::LocalStorageNamespaces, 0),
            Response::LocalStorageNamespaces {
                namespaces: vec!["@zoë/notes".into(), "core".into()]
            }
        );
        let Response::LocalStorageItems { items } = answer(
            &db,
            Request::LocalStorageItems {
                namespace: "@zoë/notes".into(),
            },
            0,
        ) else {
            panic!("no items");
        };
        let items: Vec<(&str, &str)> = items
            .iter()
            .map(|item| (item.key.as_str(), item.value.as_str()))
            .collect();
        assert_eq!(items, [("count", "3"), ("draft", "hello")]);
    }

    #[test]
    fn token_sets_are_listed_with_their_expiry_and_removed() {
        let (_dir, db) = open();
        let store = TokenStore::new(&db);
        let set = |extension: &str, provider: Option<&str>, expires_in: Option<i64>| TokenSet {
            extension_id: extension.into(),
            provider_id: provider.map(str::to_owned),
            access_token: format!("{extension}-token"),
            expires_in,
            ..TokenSet::default()
        };
        store
            .set(&set("github", Some("GitHub"), Some(3600)), 1_000)
            .unwrap();
        store.set(&set("linear", None, None), 1_000).unwrap();

        let Response::OAuthTokenSets { sets } = answer(&db, Request::OAuthTokenSets, 10_000) else {
            panic!("no sets");
        };
        let github = sets
            .iter()
            .find(|entry| entry.extension_id == "github")
            .unwrap();
        assert_eq!(github.expires_at, Some(4_600));
        assert!(
            github.expired,
            "an hour after a thousand is before ten thousand"
        );
        let linear = sets
            .iter()
            .find(|entry| entry.extension_id == "linear")
            .unwrap();
        assert!(!linear.expired, "no lifetime never expires");

        assert_eq!(
            answer(
                &db,
                Request::RemoveOAuthTokenSet {
                    extension_id: "github".into(),
                    provider_id: Some("GitHub".into()),
                },
                10_000,
            ),
            Response::Ack
        );
        let Response::OAuthTokenSets { sets } = answer(&db, Request::OAuthTokenSets, 10_000) else {
            panic!("no sets");
        };
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].extension_id, "linear");
    }
}
