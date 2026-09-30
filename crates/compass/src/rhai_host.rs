//! The services a Rhai script's granted capabilities reach, from the engine.
//!
//! The same ones an extension reaches, so a script and an extension doing the
//! same thing do it the same way:
//!
//! - the clipboard: the GNOME Shell extension's, or data-control on a wlroots
//!   compositor ([`crate::extension_runner`]'s `ShellClipboard`);
//! - opening: the default application for the target, resolved from the
//!   engine's application index and the MIME associations
//!   ([`crate::extension_apps::EngineApps`]);
//! - storage: Compass's encrypted extension database, in a namespace of the
//!   script's own ([`storage_namespace`]); the same keyring key the extensions'
//!   storage opens with, fetched on first use;
//! - notifications, over `notify-rust`.
//!
//! Every method is called on the script's blocking thread, so blocking into
//! the runtime is allowed here.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};

use compass_extension_api::CapabilityGrant;
use compass_script::{HostError, ScriptHost};
use compass_sqlcipher_sys::rusqlite::Connection;
use compass_worker_host::application_service::Apps as _;
use compass_worker_host::clipboard_service::{Clipboard as _, Content, CopyOptions};

use crate::extension_runner::{ShellClipboard, Storage};

/// Where the GNOME Shell extension's client lands once the session bus has
/// answered; shared with the engine state, which fills it.
pub type ShellSlot = Arc<RwLock<Option<Arc<compass_shell::ShellClient>>>>;

/// The local-storage namespace a script's values live in: out of reach of
/// any extension's own `<id>:data`, whatever its directory is called.
#[must_use]
pub fn storage_namespace(script: &str) -> String {
    format!("compass.rhai:{script}")
}

/// The engine's [`ScriptHost`].
pub struct EngineHost {
    shell: ShellSlot,
    handle: Option<tokio::runtime::Handle>,
    apps: Option<crate::extension_apps::EngineApps>,
    /// Where the storage database is, once asked; `None` inside means there
    /// is no keyring to open it with.
    storage: OnceLock<Option<Storage>>,
    /// Compass's data directory, whose keyring-keyed database scripts share
    /// with extensions.
    data_dir: Option<PathBuf>,
    connection: Mutex<Option<Connection>>,
}

impl std::fmt::Debug for EngineHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineHost")
            .field("data_dir", &self.data_dir)
            .finish_non_exhaustive()
    }
}

impl EngineHost {
    /// A host reaching the clipboard through `shell` (once it is filled),
    /// opening with `apps`, keeping storage in `data_dir`'s database.
    #[must_use]
    pub fn new(
        shell: ShellSlot,
        apps: Option<crate::extension_apps::EngineApps>,
        data_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            shell,
            handle: tokio::runtime::Handle::try_current().ok(),
            apps,
            storage: OnceLock::new(),
            data_dir,
            connection: Mutex::new(None),
        }
    }

    /// A host whose storage is `storage`, without asking the keyring; for
    /// tests.
    #[must_use]
    pub fn with_storage(storage: Storage) -> Self {
        let host = Self::new(ShellSlot::default(), None, None);
        let _ = host.storage.set(Some(storage));
        host
    }

    fn clipboard(&self) -> Result<ShellClipboard, HostError> {
        let shell = self
            .shell
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        ShellClipboard::available(shell, self.handle.clone())
            .ok_or_else(|| HostError::new("Compass cannot reach the clipboard in this session"))
    }

    fn storage(&self) -> Option<&Storage> {
        self.storage
            .get_or_init(|| {
                let (Some(handle), Some(dir)) = (&self.handle, &self.data_dir) else {
                    return None;
                };
                handle.block_on(crate::serve::extension_storage(dir))
            })
            .as_ref()
    }

    /// Runs `f` against the script's own namespace.
    fn with_store<T>(
        &self,
        grant: &CapabilityGrant,
        f: impl FnOnce(&compass_local_storage::Scoped<'_>) -> Result<T, compass_local_storage::Error>,
    ) -> Result<T, HostError> {
        let storage = self.storage().ok_or_else(|| {
            HostError::new("Storage needs the login keyring, and Compass cannot reach it")
        })?;
        let mut connection = self
            .connection
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if connection.is_none() {
            *connection = crate::extension_runner::open_storage(storage);
        }
        let db = connection
            .as_ref()
            .ok_or_else(|| HostError::new("Compass could not open its storage"))?;
        let local = compass_local_storage::LocalStorage::new(db);
        let scoped = local.scoped(&storage_namespace(grant.extension().as_str()));
        f(&scoped).map_err(|err| HostError::new(format!("Storage failed: {err}")))
    }
}

impl ScriptHost for EngineHost {
    fn clipboard_write(&self, _grant: CapabilityGrant, text: &str) -> Result<(), HostError> {
        self.clipboard()?
            .copy(Content::Text(text.to_owned()), CopyOptions::default());
        Ok(())
    }

    fn clipboard_read(&self, _grant: CapabilityGrant) -> Result<Option<String>, HostError> {
        let content = self.clipboard()?.read();
        Ok((!content.text.is_empty()).then_some(content.text))
    }

    fn clipboard_paste(&self, _grant: CapabilityGrant, text: &str) -> Result<(), HostError> {
        self.clipboard()?.paste(Content::Text(text.to_owned()));
        Ok(())
    }

    fn open(&self, _grant: CapabilityGrant, target: &str) -> Result<(), HostError> {
        let apps = self
            .apps
            .as_ref()
            .ok_or_else(|| HostError::new("Compass cannot open anything from here"))?;
        let opener = apps
            .default_opener(target)
            .ok_or_else(|| HostError::new(format!("No application opens {target}")))?;
        apps.launch(&opener, target);
        Ok(())
    }

    fn storage_get(&self, grant: CapabilityGrant, key: &str) -> Result<Option<String>, HostError> {
        self.with_store(&grant, |store| {
            store.get(key).map(|value| value.map(|value| value.text))
        })
    }

    fn storage_keys(&self, grant: CapabilityGrant) -> Result<Vec<String>, HostError> {
        let mut keys = self.with_store(&grant, |store| {
            store
                .list()
                .map(|values| values.into_iter().map(|(key, _)| key).collect::<Vec<_>>())
        })?;
        keys.sort();
        Ok(keys)
    }

    fn storage_set(&self, grant: CapabilityGrant, key: &str, value: &str) -> Result<(), HostError> {
        // The JSON text itself, as a string: `Value::from_json` keeps only
        // scalars, and a script stores arrays and maps.
        let value = compass_local_storage::Value {
            text: value.to_owned(),
            kind: compass_local_storage::ValueType::String,
        };
        self.with_store(&grant, |store| store.set(key, &value))
    }

    fn storage_remove(&self, grant: CapabilityGrant, key: &str) -> Result<(), HostError> {
        self.with_store(&grant, |store| store.remove(key).map(drop))
    }

    fn notify(&self, grant: CapabilityGrant, title: &str, body: &str) -> Result<(), HostError> {
        let handle = self
            .handle
            .as_ref()
            .ok_or_else(|| HostError::new("Compass cannot show notifications from here"))?;
        handle
            .block_on(
                notify_rust::Notification::new()
                    .appname("Compass")
                    .summary(title)
                    .body(body)
                    .show_async(),
            )
            .map(drop)
            .map_err(|err| {
                tracing::info!(script = %grant.extension(), %err, "notification not shown");
                HostError::new("The notification could not be shown")
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_extension_api::{Capability, CapabilityRegistry, ExtensionId};

    fn grant(
        registry: &mut CapabilityRegistry,
        id: &ExtensionId,
        cap: &Capability,
    ) -> CapabilityGrant {
        registry.declare(id, [cap.clone()]);
        registry.grant(id, cap).unwrap();
        registry.check(id, cap).unwrap()
    }

    #[test]
    fn storage_keeps_json_per_script_in_the_encrypted_database() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage {
            path: dir.path().join("storage.db"),
            key: [7; compass_crypto::KEY_SIZE],
        };
        let host = EngineHost::with_storage(storage.clone());
        let mut registry = CapabilityRegistry::new();
        let notes = ExtensionId::new("script.notes");
        let other = ExtensionId::new("script.other");
        let write = |r: &mut CapabilityRegistry, id| grant(r, id, &Capability::STORAGE_WRITE);
        let read = |r: &mut CapabilityRegistry, id| grant(r, id, &Capability::STORAGE_READ);

        host.storage_set(write(&mut registry, &notes), "notes", r#"["a","b"]"#)
            .unwrap();
        host.storage_set(write(&mut registry, &notes), "count", "2")
            .unwrap();
        assert_eq!(
            host.storage_get(read(&mut registry, &notes), "notes")
                .unwrap(),
            Some(r#"["a","b"]"#.to_owned())
        );
        assert_eq!(
            host.storage_keys(read(&mut registry, &notes)).unwrap(),
            ["count", "notes"]
        );
        assert_eq!(
            host.storage_get(read(&mut registry, &other), "notes")
                .unwrap(),
            None,
            "another script's store is its own"
        );
        host.storage_remove(write(&mut registry, &notes), "count")
            .unwrap();

        // A second host over the same database: it survives a restart.
        let again = EngineHost::with_storage(storage);
        assert_eq!(
            again.storage_keys(read(&mut registry, &notes)).unwrap(),
            ["notes"]
        );
    }

    #[test]
    fn without_services_a_call_fails_with_a_sentence_rather_than_doing_nothing() {
        let host = EngineHost::new(ShellSlot::default(), None, None);
        let mut registry = CapabilityRegistry::new();
        let id = ExtensionId::new("script.x");
        let error = host
            .open(
                grant(&mut registry, &id, &Capability::APPLICATION_OPEN),
                "https://example.org",
            )
            .unwrap_err();
        assert!(error.0.contains("cannot open"), "{error}");
        let error = host
            .storage_get(grant(&mut registry, &id, &Capability::STORAGE_READ), "k")
            .unwrap_err();
        assert!(error.0.contains("keyring"), "{error}");
    }
}
