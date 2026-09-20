//! The backend-independent boundary for credentials and ordinary local state.
//!
//! Apple passwords and backend tokens belong in the desktop Secret Service,
//! never in SQLite, configuration files, cache files, or diagnostics. This
//! module deliberately has no knowledge of rustpush or any backend protocol.

use std::{
    collections::HashMap,
    env, fmt, fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use thiserror::Error;
use zbus::{Connection, zvariant::OwnedObjectPath};

const APPLICATION_ID: &str = "litebubbles";
const SECRET_SERVICE_NAME: &str = "org.freedesktop.secrets";
const DEFAULT_COLLECTION_PATH: &str = "/org/freedesktop/secrets/collection/login";

/// XDG-owned directories for non-secret LiteBubbles state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppPaths {
    data_dir: PathBuf,
    cache_dir: PathBuf,
    config_dir: PathBuf,
}

impl AppPaths {
    /// Resolve paths from the process environment without consulting user data.
    /// XDG overrides must be absolute, as required by the XDG specification.
    pub fn from_environment() -> Result<Self, StateError> {
        let home = env::var_os("HOME").ok_or(StateError::MissingHome)?;
        let data_home = env::var_os("XDG_DATA_HOME");
        let cache_home = env::var_os("XDG_CACHE_HOME");
        let config_home = env::var_os("XDG_CONFIG_HOME");
        Self::from_values(
            Path::new(&home),
            data_home.as_deref().map(Path::new),
            cache_home.as_deref().map(Path::new),
            config_home.as_deref().map(Path::new),
        )
    }

    /// Pure path helper used by callers and deterministic tests.
    pub fn from_values(
        home: &Path,
        data_home: Option<&Path>,
        cache_home: Option<&Path>,
        config_home: Option<&Path>,
    ) -> Result<Self, StateError> {
        if !home.is_absolute() {
            return Err(StateError::RelativeHome);
        }
        Ok(Self {
            data_dir: Self::base_dir(home, data_home, ".local/share")?.join(APPLICATION_ID),
            cache_dir: Self::base_dir(home, cache_home, ".cache")?.join(APPLICATION_ID),
            config_dir: Self::base_dir(home, config_home, ".config")?.join(APPLICATION_ID),
        })
    }

    fn base_dir(
        home: &Path,
        override_dir: Option<&Path>,
        fallback: &str,
    ) -> Result<PathBuf, StateError> {
        let override_dir = override_dir.filter(|path| !path.as_os_str().is_empty());
        let path = override_dir.unwrap_or_else(|| Path::new(fallback));
        if path.is_absolute() {
            Ok(path.to_owned())
        } else if override_dir.is_some() {
            Err(StateError::RelativeXdgPath)
        } else {
            Ok(home.join(path))
        }
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// The daemon's ordinary SQLite path. This path is not a credential store.
    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("litebubbles.sqlite3")
    }

    /// Create the application data directory before opening the SQLite file.
    ///
    /// Cache and configuration directories are created by the components that
    /// first need them; the daemon must create only its durable data parent.
    pub fn ensure_data_dir(&self) -> Result<(), StateError> {
        fs::create_dir_all(&self.data_dir).map_err(|_| StateError::Create { area: "data" })
    }

    /// Remove app-owned data and cache state while retaining user preferences
    /// in the separate config directory. The caller separately deletes the
    /// account's Secret Service item through [`SecretStore`].
    pub fn purge_local_state(&self) -> Result<PurgeReport, StateError> {
        remove_directory(&self.data_dir, "data")?;
        remove_directory(&self.cache_dir, "cache")?;
        Ok(PurgeReport {
            secret_deleted: false,
            data_removed: true,
            cache_removed: true,
        })
    }
}

fn remove_directory(path: &Path, area: &'static str) -> Result<(), StateError> {
    if path.exists() {
        fs::remove_dir_all(path).map_err(|_| StateError::Purge { area })?;
    }
    Ok(())
}

/// A stable key for a Secret Service item. It contains identifiers only, never
/// the secret itself, and its formatting is intentionally redacted.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct SecretKey {
    service: String,
    account: String,
}

impl SecretKey {
    pub fn apple(account: impl Into<String>) -> Result<Self, StateError> {
        Self::new("apple", account)
    }

    pub fn new(service: impl Into<String>, account: impl Into<String>) -> Result<Self, StateError> {
        let service = service.into();
        let account = account.into();
        if service.trim().is_empty() || account.trim().is_empty() {
            return Err(StateError::InvalidSecretKey);
        }
        Ok(Self { service, account })
    }

    fn attributes(&self) -> HashMap<String, String> {
        HashMap::from([
            ("application".to_owned(), APPLICATION_ID.to_owned()),
            ("service".to_owned(), self.service.clone()),
            ("account".to_owned(), self.account.clone()),
        ])
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretKey(<redacted>)")
    }
}

impl fmt::Display for SecretKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted-secret-key>")
    }
}

/// An in-memory secret value. It intentionally omits `Debug`/`Display` data.
#[derive(Clone, Eq, PartialEq)]
pub struct SecretValue(Vec<u8>);

impl SecretValue {
    pub fn from_bytes(value: impl Into<Vec<u8>>) -> Self {
        Self(value.into())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretValue(<redacted>)")
    }
}

impl fmt::Display for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted-secret>")
    }
}

/// Secret operations supported by the backend-independent boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecretOperation {
    Get,
    Set,
    Delete,
}

impl fmt::Display for SecretOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Get => "get",
            Self::Set => "set",
            Self::Delete => "delete",
        })
    }
}

/// Structured, value-free diagnostic suitable for logs or support reports.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub component: &'static str,
    pub operation: &'static str,
    pub outcome: DiagnosticOutcome,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticOutcome {
    Started,
    Succeeded,
    Failed,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "component={} operation={} outcome={:?}",
            self.component, self.operation, self.outcome
        )
    }
}

/// Errors expose operation class only. Secret Service error strings are not
/// retained because they could accidentally echo values into logs or replies.
#[derive(Debug, Error)]
pub enum SecretStoreError {
    #[error("secret store unavailable during {operation}")]
    Unavailable { operation: SecretOperation },
    #[error("secret store rejected {operation}")]
    Rejected { operation: SecretOperation },
}

/// Async boundary for desktop-managed secrets.
#[async_trait]
pub trait SecretStore: Send + Sync {
    async fn get(&self, key: &SecretKey) -> Result<Option<SecretValue>, SecretStoreError>;
    async fn set(&self, key: &SecretKey, value: SecretValue) -> Result<(), SecretStoreError>;
    async fn delete(&self, key: &SecretKey) -> Result<bool, SecretStoreError>;
}

/// Synthetic SecretStore implementation for unit tests and credential-free
/// fixtures. It is never used as a production persistence mechanism.
#[derive(Clone, Default)]
pub struct MemorySecretStore {
    entries: Arc<Mutex<HashMap<SecretKey, SecretValue>>>,
}

impl fmt::Debug for MemorySecretStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let count = self.entries.lock().map_or(0, |entries| entries.len());
        formatter
            .debug_struct("MemorySecretStore")
            .field("entry_count", &count)
            .finish()
    }
}

#[async_trait]
impl SecretStore for MemorySecretStore {
    async fn get(&self, key: &SecretKey) -> Result<Option<SecretValue>, SecretStoreError> {
        self.entries
            .lock()
            .map_err(|_| SecretStoreError::Unavailable {
                operation: SecretOperation::Get,
            })
            .map(|entries| entries.get(key).cloned())
    }

    async fn set(&self, key: &SecretKey, value: SecretValue) -> Result<(), SecretStoreError> {
        self.entries
            .lock()
            .map_err(|_| SecretStoreError::Unavailable {
                operation: SecretOperation::Set,
            })?
            .insert(key.clone(), value);
        Ok(())
    }

    async fn delete(&self, key: &SecretKey) -> Result<bool, SecretStoreError> {
        Ok(self
            .entries
            .lock()
            .map_err(|_| SecretStoreError::Unavailable {
                operation: SecretOperation::Delete,
            })?
            .remove(key)
            .is_some())
    }
}

/// GNOME Secret Service implementation using the maintained workspace D-Bus
/// stack (`zbus` 5.12, pinned for the Rust 1.85 MSRV).
pub struct GnomeSecretService {
    connection: Connection,
}

impl fmt::Debug for GnomeSecretService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GnomeSecretService(<session-bus>)")
    }
}

impl GnomeSecretService {
    pub async fn connect() -> Result<Self, SecretStoreError> {
        Connection::session()
            .await
            .map(|connection| Self { connection })
            .map_err(|_| SecretStoreError::Unavailable {
                operation: SecretOperation::Get,
            })
    }

    async fn session(
        &self,
        operation: SecretOperation,
    ) -> Result<OwnedObjectPath, SecretStoreError> {
        let proxy = SecretServiceProxy::new(&self.connection)
            .await
            .map_err(|_| SecretStoreError::Unavailable { operation })?;
        let (_, session) = proxy
            .open_session("plain", &zbus::zvariant::Value::from(""))
            .await
            .map_err(|_| SecretStoreError::Rejected { operation })?;
        Ok(session)
    }

    async fn collection_path(
        &self,
        operation: SecretOperation,
    ) -> Result<OwnedObjectPath, SecretStoreError> {
        let proxy = SecretServiceProxy::new(&self.connection)
            .await
            .map_err(|_| SecretStoreError::Unavailable { operation })?;
        proxy
            .read_alias("default")
            .await
            .map_err(|_| SecretStoreError::Rejected { operation })
            .and_then(|path| {
                if path.as_str() == "/" {
                    OwnedObjectPath::try_from(DEFAULT_COLLECTION_PATH)
                        .map_err(|_| SecretStoreError::Rejected { operation })
                } else {
                    Ok(path)
                }
            })
    }
}

#[async_trait]
impl SecretStore for GnomeSecretService {
    async fn get(&self, key: &SecretKey) -> Result<Option<SecretValue>, SecretStoreError> {
        let session = self.session(SecretOperation::Get).await?;
        let service = SecretServiceProxy::new(&self.connection)
            .await
            .map_err(|_| SecretStoreError::Unavailable {
                operation: SecretOperation::Get,
            })?;
        let (unlocked, locked) = service.search_items(key.attributes()).await.map_err(|_| {
            SecretStoreError::Rejected {
                operation: SecretOperation::Get,
            }
        })?;
        let Some(item_path) = unlocked.into_iter().chain(locked).next() else {
            let _ = service.close_session(&session).await;
            return Ok(None);
        };
        let item = SecretItemProxy::builder(&self.connection)
            .destination(SECRET_SERVICE_NAME)
            .map_err(|_| SecretStoreError::Unavailable {
                operation: SecretOperation::Get,
            })?
            .path(item_path)
            .map_err(|_| SecretStoreError::Rejected {
                operation: SecretOperation::Get,
            })?
            .build()
            .await
            .map_err(|_| SecretStoreError::Unavailable {
                operation: SecretOperation::Get,
            })?;
        let result = item
            .get_secret(&session)
            .await
            .map(|secret| SecretValue::from_bytes(secret.2))
            .map_err(|_| SecretStoreError::Rejected {
                operation: SecretOperation::Get,
            });
        let _ = service.close_session(&session).await;
        result.map(Some)
    }

    async fn set(&self, key: &SecretKey, value: SecretValue) -> Result<(), SecretStoreError> {
        let session = self.session(SecretOperation::Set).await?;
        let service = SecretServiceProxy::new(&self.connection)
            .await
            .map_err(|_| SecretStoreError::Unavailable {
                operation: SecretOperation::Set,
            })?;
        let (mut unlocked, locked) =
            service.search_items(key.attributes()).await.map_err(|_| {
                SecretStoreError::Rejected {
                    operation: SecretOperation::Set,
                }
            })?;
        unlocked.extend(locked);
        let secret = (
            session.clone(),
            Vec::new(),
            value.into_bytes(),
            "text/plain".to_owned(),
        );
        if let Some(item_path) = unlocked.into_iter().next() {
            let item = SecretItemProxy::builder(&self.connection)
                .destination(SECRET_SERVICE_NAME)
                .map_err(|_| SecretStoreError::Unavailable {
                    operation: SecretOperation::Set,
                })?
                .path(item_path)
                .map_err(|_| SecretStoreError::Rejected {
                    operation: SecretOperation::Set,
                })?
                .build()
                .await
                .map_err(|_| SecretStoreError::Unavailable {
                    operation: SecretOperation::Set,
                })?;
            item.set_secret(secret)
                .await
                .map_err(|_| SecretStoreError::Rejected {
                    operation: SecretOperation::Set,
                })?;
        } else {
            let collection_path = self.collection_path(SecretOperation::Set).await?;
            let collection = SecretCollectionProxy::builder(&self.connection)
                .destination(SECRET_SERVICE_NAME)
                .map_err(|_| SecretStoreError::Unavailable {
                    operation: SecretOperation::Set,
                })?
                .path(collection_path)
                .map_err(|_| SecretStoreError::Rejected {
                    operation: SecretOperation::Set,
                })?
                .build()
                .await
                .map_err(|_| SecretStoreError::Unavailable {
                    operation: SecretOperation::Set,
                })?;
            collection
                .create_item(item_properties(key)?, secret, true)
                .await
                .map_err(|_| SecretStoreError::Rejected {
                    operation: SecretOperation::Set,
                })?;
        }
        let _ = service.close_session(&session).await;
        Ok(())
    }

    async fn delete(&self, key: &SecretKey) -> Result<bool, SecretStoreError> {
        let session = self.session(SecretOperation::Delete).await?;
        let service = SecretServiceProxy::new(&self.connection)
            .await
            .map_err(|_| SecretStoreError::Unavailable {
                operation: SecretOperation::Delete,
            })?;
        let (mut unlocked, locked) =
            service.search_items(key.attributes()).await.map_err(|_| {
                SecretStoreError::Rejected {
                    operation: SecretOperation::Delete,
                }
            })?;
        unlocked.extend(locked);
        let Some(item_path) = unlocked.into_iter().next() else {
            let _ = service.close_session(&session).await;
            return Ok(false);
        };
        let item = SecretItemProxy::builder(&self.connection)
            .destination(SECRET_SERVICE_NAME)
            .map_err(|_| SecretStoreError::Unavailable {
                operation: SecretOperation::Delete,
            })?
            .path(item_path)
            .map_err(|_| SecretStoreError::Rejected {
                operation: SecretOperation::Delete,
            })?
            .build()
            .await
            .map_err(|_| SecretStoreError::Unavailable {
                operation: SecretOperation::Delete,
            })?;
        let _prompt = item
            .delete()
            .await
            .map_err(|_| SecretStoreError::Rejected {
                operation: SecretOperation::Delete,
            })?;
        let _ = service.close_session(&session).await;
        Ok(true)
    }
}

fn item_properties(
    key: &SecretKey,
) -> Result<HashMap<String, zbus::zvariant::OwnedValue>, SecretStoreError> {
    let attributes = zbus::zvariant::OwnedValue::from(key.attributes());
    Ok(HashMap::from([
        (
            "org.freedesktop.Secret.Item.Label".to_owned(),
            zbus::zvariant::OwnedValue::try_from(zbus::zvariant::Value::from(
                "LiteBubbles credential",
            ))
            .map_err(|_| SecretStoreError::Rejected {
                operation: SecretOperation::Set,
            })?,
        ),
        (
            "org.freedesktop.Secret.Item.Attributes".to_owned(),
            attributes,
        ),
    ]))
}

#[zbus::proxy(
    interface = "org.freedesktop.Secret.Service",
    default_service = "org.freedesktop.secrets",
    default_path = "/org/freedesktop/secrets"
)]
trait SecretService {
    async fn open_session(
        &self,
        algorithm: &str,
        input: &zbus::zvariant::Value<'_>,
    ) -> zbus::Result<(zbus::zvariant::OwnedValue, OwnedObjectPath)>;
    async fn search_items(
        &self,
        attributes: HashMap<String, String>,
    ) -> zbus::Result<(Vec<OwnedObjectPath>, Vec<OwnedObjectPath>)>;
    async fn read_alias(&self, name: &str) -> zbus::Result<OwnedObjectPath>;
    async fn close_session(&self, session: &OwnedObjectPath) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.Secret.Collection",
    default_service = "org.freedesktop.secrets"
)]
trait SecretCollection {
    async fn create_item(
        &self,
        properties: HashMap<String, zbus::zvariant::OwnedValue>,
        secret: (OwnedObjectPath, Vec<u8>, Vec<u8>, String),
        replace: bool,
    ) -> zbus::Result<(OwnedObjectPath, OwnedObjectPath)>;
}

#[zbus::proxy(
    interface = "org.freedesktop.Secret.Item",
    default_service = "org.freedesktop.secrets"
)]
trait SecretItem {
    /// The Secret Service protocol scopes reads to the opened session path.
    async fn get_secret(
        &self,
        session: &OwnedObjectPath,
    ) -> zbus::Result<(OwnedObjectPath, Vec<u8>, Vec<u8>, String)>;
    async fn set_secret(
        &self,
        secret: (OwnedObjectPath, Vec<u8>, Vec<u8>, String),
    ) -> zbus::Result<()>;
    async fn delete(&self) -> zbus::Result<OwnedObjectPath>;
}

/// Result of a sign-out/purge operation. It contains counts/flags only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PurgeReport {
    pub secret_deleted: bool,
    pub data_removed: bool,
    pub cache_removed: bool,
}

/// Session lifecycle boundary. Backend/session adapters can use this without
/// learning how credentials or local SQLite state are physically stored.
pub struct SessionState<S> {
    paths: AppPaths,
    secrets: S,
}

impl<S> SessionState<S>
where
    S: SecretStore,
{
    pub fn new(paths: AppPaths, secrets: S) -> Self {
        Self { paths, secrets }
    }

    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }

    pub async fn sign_out(&self, key: &SecretKey) -> Result<PurgeReport, StateError> {
        let secret_deleted = self
            .secrets
            .delete(key)
            .await
            .map_err(StateError::SecretStore)?;
        let mut report = self.paths.purge_local_state()?;
        report.secret_deleted = secret_deleted;
        Ok(report)
    }
}

#[derive(Debug, Error)]
pub enum StateError {
    #[error("HOME is required to resolve XDG paths")]
    MissingHome,
    #[error("HOME must be an absolute path")]
    RelativeHome,
    #[error("XDG base directories must be absolute paths")]
    RelativeXdgPath,
    #[error("secret key must contain non-empty identifiers")]
    InvalidSecretKey,
    #[error("failed to purge {area} state")]
    Purge { area: &'static str },
    #[error("failed to create {area} state directory")]
    Create { area: &'static str },
    #[error("secret operation failed: {0}")]
    SecretStore(#[source] SecretStoreError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_lite::future::block_on;
    use tempfile::tempdir;

    #[test]
    fn paths_keep_data_cache_and_config_separate() {
        let paths = AppPaths::from_values(
            Path::new("/synthetic/home"),
            Some(Path::new("/synthetic/data")),
            Some(Path::new("/synthetic/cache")),
            Some(Path::new("/synthetic/config")),
        )
        .expect("paths");
        assert_eq!(
            paths.database_path(),
            Path::new("/synthetic/data/litebubbles/litebubbles.sqlite3")
        );
        assert_eq!(paths.cache_dir(), Path::new("/synthetic/cache/litebubbles"));
        assert_eq!(
            paths.config_dir(),
            Path::new("/synthetic/config/litebubbles")
        );
        assert_ne!(paths.data_dir(), paths.cache_dir());
        assert_ne!(paths.data_dir(), paths.config_dir());
    }

    #[test]
    fn default_paths_use_home_when_xdg_overrides_are_absent() {
        let paths =
            AppPaths::from_values(Path::new("/synthetic/home"), None, None, None).expect("paths");
        assert_eq!(
            paths.database_path(),
            Path::new("/synthetic/home/.local/share/litebubbles/litebubbles.sqlite3")
        );
        assert_eq!(
            paths.cache_dir(),
            Path::new("/synthetic/home/.cache/litebubbles")
        );
        assert_eq!(
            paths.config_dir(),
            Path::new("/synthetic/home/.config/litebubbles")
        );
    }

    #[test]
    fn ensure_data_dir_creates_only_the_durable_state_parent() {
        let root = tempdir().expect("tempdir");
        let data_home = root.path().join("data");
        let cache_home = root.path().join("cache");
        let config_home = root.path().join("config");
        let paths = AppPaths::from_values(
            root.path(),
            Some(&data_home),
            Some(&cache_home),
            Some(&config_home),
        )
        .expect("paths");

        paths.ensure_data_dir().expect("data directory");

        assert!(paths.data_dir().is_dir());
        assert!(!paths.cache_dir().exists());
        assert!(!paths.config_dir().exists());
    }

    #[test]
    fn relative_xdg_override_is_rejected() {
        let error = AppPaths::from_values(
            Path::new("/synthetic/home"),
            Some(Path::new("relative-data")),
            None,
            None,
        )
        .expect_err("relative override must not be used");
        assert_eq!(
            error.to_string(),
            "XDG base directories must be absolute paths"
        );
    }

    #[test]
    fn secret_debug_display_and_diagnostics_are_redacted() {
        let key = SecretKey::apple("synthetic-account").expect("key");
        let value = SecretValue::from_bytes(b"synthetic-secret".to_vec());
        let diagnostic = Diagnostic {
            component: "credential-boundary",
            operation: "set",
            outcome: DiagnosticOutcome::Failed,
        };
        for rendered in [
            format!("{key:?}"),
            key.to_string(),
            format!("{value:?}"),
            value.to_string(),
            diagnostic.to_string(),
        ] {
            assert!(!rendered.contains("synthetic-secret"));
            assert!(!rendered.contains("synthetic-account"));
        }
    }

    #[test]
    fn memory_secret_store_round_trips_and_deletes() {
        let key = SecretKey::apple("synthetic-account").expect("key");
        let store = MemorySecretStore::default();
        block_on(store.set(&key, SecretValue::from_bytes(b"synthetic-secret".to_vec())))
            .expect("set");
        assert_eq!(
            block_on(store.get(&key))
                .expect("get")
                .expect("secret")
                .as_bytes(),
            b"synthetic-secret"
        );
        assert!(block_on(store.delete(&key)).expect("delete"));
        assert!(
            block_on(store.get(&key))
                .expect("get after delete")
                .is_none()
        );
    }

    #[test]
    fn sign_out_deletes_secret_and_purges_data_and_cache_only() {
        let root = tempdir().expect("tempdir");
        let data_home = root.path().join("data");
        let cache_home = root.path().join("cache");
        let config_home = root.path().join("config");
        let paths = AppPaths::from_values(
            root.path(),
            Some(&data_home),
            Some(&cache_home),
            Some(&config_home),
        )
        .expect("paths");
        fs::create_dir_all(paths.data_dir()).expect("data");
        fs::create_dir_all(paths.cache_dir()).expect("cache");
        fs::create_dir_all(paths.config_dir()).expect("config");
        fs::write(paths.data_dir().join("state"), b"synthetic-state").expect("state");
        fs::write(paths.cache_dir().join("cache"), b"synthetic-cache").expect("cache");
        fs::write(paths.config_dir().join("settings"), b"synthetic-settings").expect("settings");

        let key = SecretKey::apple("synthetic-account").expect("key");
        let store = MemorySecretStore::default();
        block_on(store.set(&key, SecretValue::from_bytes(b"synthetic-secret".to_vec())))
            .expect("set");
        let session = SessionState::new(paths.clone(), store.clone());
        let report = block_on(session.sign_out(&key)).expect("sign out");

        assert_eq!(
            report,
            PurgeReport {
                secret_deleted: true,
                data_removed: true,
                cache_removed: true
            }
        );
        assert!(!paths.data_dir().exists());
        assert!(!paths.cache_dir().exists());
        assert!(paths.config_dir().join("settings").exists());
        assert!(
            block_on(store.get(&key))
                .expect("get after sign out")
                .is_none()
        );
    }
}
