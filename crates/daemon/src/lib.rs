//! Backend-independent daemon service boundary.
//!
//! The service owns the protocol-facing state and persistence.  It does not
//! know about GTK or rustpush.  A backend is supplied through [`BackendAdapter`]
//! and can be unavailable while the local, read-only service remains useful.

use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use litebubbles_core as core;
use litebubbles_protocol as protocol;
use litebubbles_protocol::{
    AccountState, AttachmentKind, AttachmentReference, ConversationPage, ConversationPageRequest,
    DeliveryState, Device, EventKind, EventMetadata, HistoryPage, HistoryPageRequest, Id, Message,
    MessagePart, MessagePartKind, NotificationMode, ProtocolError, ProtocolEvent, ReadState,
    RefreshRequest, RefreshResponse, RefreshState, SendMessageRequest, SendMessageResponse,
    Settings, SyncChanged, SyncState, Timestamp, UpdateSettingsRequest,
};
use litebubbles_rustpush_backend::{
    LiveError, LiveSession, MacHardwareInput, SessionSnapshot, TwoFactorPrompt,
    initialize_local_keystore,
};
use litebubbles_storage::{AppPaths, GnomeSecretService, SecretKey, SecretStore, SecretValue};
use litebubbles_storage::{MessageCursor, MessagePage, StorageError, Store};
use rand::RngCore;
use signal_hook::{
    consts::signal::{SIGINT, SIGTERM},
    iterator::Signals,
};
use thiserror::Error;
use tracing::{debug, error, info, warn};
use zbus::object_server::SignalEmitter;

/// Exact blocker shared by service errors, sync state, and documentation.
pub const RUSTPUSH_BLOCKER: &str = "live rustpush synchronization is not connected to the general D-Bus service yet; run litebubblesd setup before using the production rustpush path";

/// User-facing setup/send errors deliberately contain no credentials, tokens,
/// hardware payloads, or message bodies.
#[derive(Debug, Error)]
pub enum ProductionCommandError {
    #[error(
        "Apple compatibility setup is required. Run scripts/setup-production.sh and provide an official supported OpenBubbles release."
    )]
    MissingCompatibility,
    #[error("the Apple account identifier is required")]
    MissingAccount,
    #[error("the saved Mac activation input is missing; run litebubblesd setup first")]
    MissingHardware,
    #[error("the saved Apple session is missing; run litebubblesd setup first")]
    MissingSession,
    #[error("the saved Apple session or hardware input is invalid")]
    InvalidSavedState,
    #[error("the supplied Mac activation input is invalid: {0}")]
    InvalidHardware(String),
    #[error("the Secret Service is unavailable for LiteBubbles setup")]
    SecretStore,
    #[error("the local Apple session could not be prepared")]
    LocalState,
    #[error("Apple setup failed: {0}")]
    Backend(String),
}

fn production_key(
    service: &'static str,
    account: &str,
) -> Result<SecretKey, ProductionCommandError> {
    if account.trim().is_empty() {
        return Err(ProductionCommandError::MissingAccount);
    }
    SecretKey::new(service, account).map_err(|_| ProductionCommandError::MissingAccount)
}

async fn production_secrets() -> Result<GnomeSecretService, ProductionCommandError> {
    GnomeSecretService::connect()
        .await
        .map_err(|_| ProductionCommandError::SecretStore)
}

async fn get_production_secret(
    secrets: &GnomeSecretService,
    key: &SecretKey,
) -> Result<Option<Vec<u8>>, ProductionCommandError> {
    secrets
        .get(key)
        .await
        .map(|value| value.map(SecretValue::into_bytes))
        .map_err(|_| ProductionCommandError::SecretStore)
}

async fn ensure_keystore_key(
    secrets: &GnomeSecretService,
    account: &str,
) -> Result<[u8; 32], ProductionCommandError> {
    let key = production_key("rustpush-keystore", account)?;
    if let Some(value) = get_production_secret(secrets, &key).await? {
        return value
            .try_into()
            .map_err(|_| ProductionCommandError::InvalidSavedState);
    }
    let mut value = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut value);
    secrets
        .set(&key, SecretValue::from_bytes(value.to_vec()))
        .await
        .map_err(|_| ProductionCommandError::SecretStore)?;
    Ok(value)
}

async fn prepare_production_state(
    account: &str,
) -> Result<(AppPaths, GnomeSecretService), ProductionCommandError> {
    let paths = AppPaths::from_environment().map_err(|_| ProductionCommandError::LocalState)?;
    paths
        .ensure_data_dir()
        .map_err(|_| ProductionCommandError::LocalState)?;
    fs::create_dir_all(paths.cache_dir()).map_err(|_| ProductionCommandError::LocalState)?;
    let secrets = production_secrets().await?;
    let keystore_key = ensure_keystore_key(&secrets, account).await?;
    initialize_local_keystore(
        paths.data_dir().join("rustpush/keystore.plist"),
        keystore_key,
    )
    .map_err(|_| ProductionCommandError::LocalState)?;
    Ok((paths, secrets))
}

fn backend_error(error: LiveError) -> ProductionCommandError {
    match error {
        LiveError::Validation(_) => ProductionCommandError::MissingCompatibility,
        other => ProductionCommandError::Backend(other.to_string()),
    }
}

/// Run the first-time production setup path. The caller supplies the password
/// and a callback for 2FA so neither is placed in process arguments or logs.
pub async fn run_production_setup<F>(
    account: String,
    hardware_payload: String,
    password: String,
    mut prompt: F,
) -> Result<(), ProductionCommandError>
where
    F: FnMut(TwoFactorPrompt) -> String,
{
    let hardware = MacHardwareInput::parse_base64(&hardware_payload)
        .map_err(|error| ProductionCommandError::InvalidHardware(error.to_string()))?;
    let (paths, secrets) = prepare_production_state(&account).await?;
    let session_key = production_key("apple-session", &account)?;
    let existing = get_production_secret(&secrets, &session_key).await?;
    let session = LiveSession::connect(
        hardware,
        existing.as_deref(),
        &account,
        Some(&password),
        paths.data_dir().join("anisette"),
        paths.cache_dir().join("ids-key-cache.plist"),
        &mut prompt,
    )
    .await
    .map_err(backend_error)?;
    let snapshot = session.snapshot().await.map_err(backend_error)?;
    secrets
        .set(&session_key, SecretValue::from_bytes(snapshot.into_bytes()))
        .await
        .map_err(|_| ProductionCommandError::SecretStore)?;
    let hardware_key = production_key("apple-hardware", &account)?;
    secrets
        .set(
            &hardware_key,
            SecretValue::from_bytes(hardware_payload.trim().as_bytes().to_vec()),
        )
        .await
        .map_err(|_| ProductionCommandError::SecretStore)?;
    Ok(())
}

async fn load_live_session(
    account: &str,
) -> Result<(AppPaths, GnomeSecretService, LiveSession, SecretKey), ProductionCommandError> {
    let (paths, secrets) = prepare_production_state(account).await?;
    let hardware_key = production_key("apple-hardware", account)?;
    let hardware_payload = get_production_secret(&secrets, &hardware_key)
        .await?
        .ok_or(ProductionCommandError::MissingHardware)?;
    let hardware_payload = String::from_utf8(hardware_payload)
        .map_err(|_| ProductionCommandError::InvalidSavedState)?;
    let hardware = MacHardwareInput::parse_base64(&hardware_payload)
        .map_err(|_| ProductionCommandError::InvalidSavedState)?;
    let session_key = production_key("apple-session", account)?;
    let snapshot = get_production_secret(&secrets, &session_key)
        .await?
        .ok_or(ProductionCommandError::MissingSession)?;
    let snapshot = SessionSnapshot::from_bytes(snapshot)
        .map_err(|_| ProductionCommandError::InvalidSavedState)?;
    let session = LiveSession::connect(
        hardware,
        Some(snapshot.as_bytes()),
        account,
        None,
        paths.data_dir().join("anisette"),
        paths.cache_dir().join("ids-key-cache.plist"),
        &mut |_| String::new(),
    )
    .await
    .map_err(backend_error)?;
    Ok((paths, secrets, session, session_key))
}

pub async fn run_production_send(
    account: String,
    destination: String,
    text: String,
) -> Result<(), ProductionCommandError> {
    let (_paths, secrets, session, session_key) = load_live_session(&account).await?;
    session
        .send_text(&destination, &text)
        .await
        .map_err(backend_error)?;
    let snapshot = session.snapshot().await.map_err(backend_error)?;
    secrets
        .set(&session_key, SecretValue::from_bytes(snapshot.into_bytes()))
        .await
        .map_err(|_| ProductionCommandError::SecretStore)
}

pub async fn run_production_listen(
    account: String,
    timeout: std::time::Duration,
) -> Result<(), ProductionCommandError> {
    let (_paths, secrets, session, session_key) = load_live_session(&account).await?;
    session
        .wait_for_message(timeout)
        .await
        .map_err(backend_error)?;
    let snapshot = session.snapshot().await.map_err(backend_error)?;
    secrets
        .set(&session_key, SecretValue::from_bytes(snapshot.into_bytes()))
        .await
        .map_err(|_| ProductionCommandError::SecretStore)
}

/// Errors returned by a backend adapter.  These deliberately contain no
/// account identifiers, message bodies, credentials, or remote addresses.
#[derive(Debug, Error)]
pub enum AdapterError {
    /// The live backend cannot be used in this build.
    #[error("{RUSTPUSH_BLOCKER}")]
    Unavailable,
    /// The adapter has not implemented a requested operation yet.
    #[error("backend operation is not implemented")]
    Unsupported,
}

/// Seam between the daemon protocol service and a live or test backend.
///
/// LB-010 intentionally provides the unavailable implementation below rather
/// than pretending to implement rustpush.  A later live-sync issue can add a
/// real adapter without changing the D-Bus service boundary.
#[async_trait]
pub trait BackendAdapter: Send + Sync {
    /// Request a backend refresh for the supplied scope.
    async fn refresh(&self, request: RefreshRequest) -> Result<(), AdapterError>;

    /// Submit a message to the backend.
    async fn send_message(
        &self,
        request: SendMessageRequest,
        accepted_at: Timestamp,
    ) -> Result<SendMessageResponse, AdapterError>;
}

/// Explicit adapter used by the production daemon until rustpush is unblocked.
#[derive(Debug, Default)]
pub struct UnavailableBackend;

#[async_trait]
impl BackendAdapter for UnavailableBackend {
    async fn refresh(&self, _request: RefreshRequest) -> Result<(), AdapterError> {
        Err(AdapterError::Unavailable)
    }

    async fn send_message(
        &self,
        _request: SendMessageRequest,
        _accepted_at: Timestamp,
    ) -> Result<SendMessageResponse, AdapterError> {
        Err(AdapterError::Unavailable)
    }
}

/// Clock abstraction used to make service tests deterministic.
pub trait Clock: Send + Sync {
    /// Return milliseconds since the Unix epoch.
    fn now(&self) -> Timestamp;
}

#[derive(Debug, Default)]
struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| {
                duration.as_millis().min(i64::MAX as u128) as i64
            });
        Timestamp(millis)
    }
}

/// Fixed clock useful for credential-free fixtures and protocol tests.
#[derive(Debug)]
pub struct FixedClock(pub i64);

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.0)
    }
}

/// Async-safe service interface used by the D-Bus object and non-D-Bus tests.
#[async_trait]
pub trait DaemonService: Send + Sync {
    async fn hello(
        &self,
        request: protocol::HelloRequest,
    ) -> Result<protocol::HelloResponse, ProtocolError>;
    async fn get_account_state(&self) -> Result<AccountState, ProtocolError>;
    async fn get_conversations(
        &self,
        request: ConversationPageRequest,
    ) -> Result<ConversationPage, ProtocolError>;
    async fn get_history(&self, request: HistoryPageRequest) -> Result<HistoryPage, ProtocolError>;
    async fn send_message(
        &self,
        request: SendMessageRequest,
    ) -> Result<SendMessageResponse, ProtocolError>;
    async fn get_attachment(&self, attachment_id: Id)
    -> Result<AttachmentReference, ProtocolError>;
    async fn get_settings(&self) -> Result<Settings, ProtocolError>;
    async fn update_settings(
        &self,
        request: UpdateSettingsRequest,
    ) -> Result<Settings, ProtocolError>;
    async fn refresh(&self, request: RefreshRequest) -> Result<RefreshResponse, ProtocolError>;

    /// Drain typed events generated by service operations for D-Bus emission.
    fn drain_events(&self) -> Vec<ProtocolEvent>;
}

struct ServiceInner<A> {
    store: Mutex<Store>,
    settings: Mutex<Settings>,
    events: Mutex<Vec<ProtocolEvent>>,
    next_event: std::sync::atomic::AtomicU64,
    next_operation: std::sync::atomic::AtomicU64,
    adapter: A,
    clock: Arc<dyn Clock>,
}

/// Storage-backed daemon state.  SQLite work is kept behind a mutex and no
/// lock is held over an await point, making calls safe to dispatch concurrently.
pub struct ServiceState<A = UnavailableBackend> {
    inner: Arc<ServiceInner<A>>,
}

impl ServiceState<UnavailableBackend> {
    /// Creates production-shaped state with the explicit unavailable adapter.
    pub fn new(store: Store) -> Self {
        Self::with_adapter_and_clock(store, UnavailableBackend, Arc::new(SystemClock))
    }
}

impl<A> ServiceState<A>
where
    A: BackendAdapter,
{
    /// Creates state with a caller-supplied adapter and system clock.
    pub fn with_adapter(store: Store, adapter: A) -> Self {
        Self::with_adapter_and_clock(store, adapter, Arc::new(SystemClock))
    }

    /// Creates state with fully controlled backend and clock dependencies.
    pub fn with_adapter_and_clock(store: Store, adapter: A, clock: Arc<dyn Clock>) -> Self {
        Self {
            inner: Arc::new(ServiceInner {
                store: Mutex::new(store),
                settings: Mutex::new(Settings {
                    notifications: NotificationMode::All,
                    show_previews: true,
                    allow_local_attachment_locations: false,
                }),
                events: Mutex::new(Vec::new()),
                next_event: std::sync::atomic::AtomicU64::new(1),
                next_operation: std::sync::atomic::AtomicU64::new(1),
                adapter,
                clock,
            }),
        }
    }

    fn storage<T>(
        &self,
        operation: impl FnOnce(&Store) -> Result<T, StorageError>,
    ) -> Result<T, ProtocolError> {
        let store = self
            .inner
            .store
            .lock()
            .map_err(|_| ProtocolError::Internal("local storage lock failed".to_owned()))?;
        operation(&store).map_err(storage_error)
    }

    fn event_metadata(&self, occurred_at: Timestamp) -> EventMetadata {
        let number = self
            .inner
            .next_event
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        EventMetadata {
            event_id: Id::new(format!("event-{number:06}"))
                .expect("generated event ID is non-empty"),
            occurred_at,
        }
    }

    fn queue_event(&self, event: ProtocolEvent) {
        if let Ok(mut events) = self.inner.events.lock() {
            events.push(event);
        } else {
            error!("event queue lock failed during daemon shutdown");
        }
    }

    fn account(&self) -> Result<core::Account, ProtocolError> {
        self.storage(|store| {
            store
                .list_accounts()?
                .into_iter()
                .next()
                .ok_or_else(|| StorageError::Corrupt {
                    entity: "account",
                    message: "no account configured".to_owned(),
                })
        })
        .map_err(|error| match error {
            ProtocolError::Internal(_) => {
                ProtocolError::NotReady("no account configured".to_owned())
            }
            other => other,
        })
    }

    fn account_state(&self) -> Result<AccountState, ProtocolError> {
        let account = self.account()?;
        Ok(AccountState {
            account_id: id(account.id.as_str())?,
            display_name: account.display_name,
            identities: account
                .identities
                .iter()
                .map(identity)
                .collect::<Result<_, _>>()?,
            devices: account
                .devices
                .iter()
                .map(device)
                .collect::<Result<_, _>>()?,
            sync: SyncState {
                connected: false,
                initial_sync_complete: true,
                last_success_at: None,
                error: Some(RUSTPUSH_BLOCKER.to_owned()),
            },
        })
    }

    fn sync_event(&self, operation_id: Option<Id>, state: SyncState, at: Timestamp) {
        self.queue_event(ProtocolEvent {
            kind: EventKind::SyncChanged,
            account: None,
            conversation: None,
            message: None,
            reaction: None,
            typing: None,
            read: None,
            sync: Some(SyncChanged {
                metadata: self.event_metadata(at),
                operation_id,
                state,
            }),
            attachment: None,
            call: None,
            find_my: None,
        });
    }

    fn operation_id(&self) -> Id {
        let number = self
            .inner
            .next_operation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Id::new(format!("refresh-{number:06}")).expect("generated operation ID is non-empty")
    }
}

#[async_trait]
impl<A> DaemonService for ServiceState<A>
where
    A: BackendAdapter,
{
    async fn hello(
        &self,
        request: protocol::HelloRequest,
    ) -> Result<protocol::HelloResponse, ProtocolError> {
        let version = protocol::negotiate_version(&request)?;
        debug!(version, "protocol hello negotiated");
        Ok(protocol::HelloResponse {
            version,
            server_name: "litebubblesd".to_owned(),
            capabilities: vec![
                "events".to_owned(),
                "bodyless-attachments".to_owned(),
                "storage-backed-state".to_owned(),
            ],
        })
    }

    async fn get_account_state(&self) -> Result<AccountState, ProtocolError> {
        self.account_state()
    }

    async fn get_conversations(
        &self,
        request: ConversationPageRequest,
    ) -> Result<ConversationPage, ProtocolError> {
        ConversationPageRequest::new(request.cursor.clone(), request.limit)?;
        let conversations = self.storage(Store::list_conversations)?;
        let start = parse_index_cursor(request.cursor.as_deref())?;
        if start > conversations.len() {
            return Err(ProtocolError::InvalidRequest(
                "conversation cursor is out of range".to_owned(),
            ));
        }
        let end = start
            .saturating_add(request.limit as usize)
            .min(conversations.len());
        let page = conversations[start..end]
            .iter()
            .map(|value| self.protocol_conversation(value))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ConversationPage {
            conversations: page,
            next_cursor: (end < conversations.len()).then(|| end.to_string()),
        })
    }

    async fn get_history(&self, request: HistoryPageRequest) -> Result<HistoryPage, ProtocolError> {
        HistoryPageRequest::new(
            request.conversation_id.clone(),
            request.cursor.clone(),
            request.limit,
        )?;
        let conversation_id = core::ConversationId::try_from(request.conversation_id.0.clone())
            .map_err(|_| ProtocolError::InvalidRequest("conversation ID is invalid".to_owned()))?;
        self.storage(|store| store.get_conversation(&conversation_id))?
            .ok_or_else(|| ProtocolError::NotFound(request.conversation_id.to_string()))?;
        let mut page = MessagePage::new(request.limit).map_err(storage_error)?;
        if let Some(cursor) = request.cursor.as_deref() {
            page = page.after(parse_message_cursor(cursor)?);
        }
        let messages = self.storage(|store| store.list_messages(&conversation_id, page))?;
        let next_cursor = (messages.len() == request.limit as usize)
            .then(|| messages.last())
            .flatten()
            .map(|message| format!("{}:{}", message.sent_at.as_millis(), message.id));
        Ok(HistoryPage {
            messages: messages
                .iter()
                .map(|value| self.protocol_message(value))
                .collect::<Result<Vec<_>, _>>()?,
            next_cursor,
        })
    }

    async fn send_message(
        &self,
        request: SendMessageRequest,
    ) -> Result<SendMessageResponse, ProtocolError> {
        for part in &request.parts {
            if let Some(attachment) = &part.attachment {
                attachment.validate()?;
            }
        }
        self.inner
            .adapter
            .send_message(request, self.inner.clock.now())
            .await
            .map_err(adapter_error)
    }

    async fn get_attachment(
        &self,
        attachment_id: Id,
    ) -> Result<AttachmentReference, ProtocolError> {
        let core_id = core::AttachmentId::try_from(attachment_id.0.clone())
            .map_err(|_| ProtocolError::InvalidRequest("attachment ID is invalid".to_owned()))?;
        let attachment = self
            .storage(|store| store.get_attachment(&core_id))?
            .ok_or_else(|| ProtocolError::NotFound(attachment_id.to_string()))?;
        let allow_location = self
            .inner
            .settings
            .lock()
            .map_err(|_| ProtocolError::Internal("settings lock failed".to_owned()))?
            .allow_local_attachment_locations;
        protocol_attachment(&attachment, allow_location)
    }

    async fn get_settings(&self) -> Result<Settings, ProtocolError> {
        self.inner
            .settings
            .lock()
            .map(|settings| settings.clone())
            .map_err(|_| ProtocolError::Internal("settings lock failed".to_owned()))
    }

    async fn update_settings(
        &self,
        request: UpdateSettingsRequest,
    ) -> Result<Settings, ProtocolError> {
        let mut settings = self
            .inner
            .settings
            .lock()
            .map_err(|_| ProtocolError::Internal("settings lock failed".to_owned()))?;
        if let Some(value) = request.notifications {
            settings.notifications = value;
        }
        if let Some(value) = request.show_previews {
            settings.show_previews = value;
        }
        if let Some(value) = request.allow_local_attachment_locations {
            settings.allow_local_attachment_locations = value;
        }
        Ok(settings.clone())
    }

    async fn refresh(&self, request: RefreshRequest) -> Result<RefreshResponse, ProtocolError> {
        if let Some(account_id) = &request.account_id {
            let id = core::AccountId::try_from(account_id.0.clone())
                .map_err(|_| ProtocolError::InvalidRequest("account ID is invalid".to_owned()))?;
            if self.storage(|store| store.get_account(&id))?.is_none() {
                return Err(ProtocolError::NotFound(account_id.to_string()));
            }
        }
        let operation_id = self.operation_id();
        let at = self.inner.clock.now();
        let result = self.inner.adapter.refresh(request).await;
        match result {
            Ok(()) => {
                self.sync_event(
                    Some(operation_id.clone()),
                    SyncState {
                        connected: true,
                        initial_sync_complete: true,
                        last_success_at: Some(at),
                        error: None,
                    },
                    at,
                );
                Ok(RefreshResponse {
                    operation_id,
                    state: RefreshState::Complete,
                })
            }
            Err(error) => {
                warn!("refresh unavailable: {error}");
                self.sync_event(
                    Some(operation_id.clone()),
                    SyncState {
                        connected: false,
                        initial_sync_complete: false,
                        last_success_at: None,
                        error: Some(RUSTPUSH_BLOCKER.to_owned()),
                    },
                    at,
                );
                Ok(RefreshResponse {
                    operation_id,
                    state: RefreshState::Failed,
                })
            }
        }
    }

    fn drain_events(&self) -> Vec<ProtocolEvent> {
        self.inner
            .events
            .lock()
            .map(|mut events| std::mem::take(&mut *events))
            .unwrap_or_default()
    }
}

impl<A> ServiceState<A>
where
    A: BackendAdapter,
{
    fn protocol_conversation(
        &self,
        conversation: &core::Conversation,
    ) -> Result<protocol::Conversation, ProtocolError> {
        let unread_count = self
            .storage(|store| store.unread_state(&conversation.id))?
            .map_or(0, |state| state.unread_count.min(u32::MAX as u64) as u32);
        Ok(protocol::Conversation {
            id: id(conversation.id.as_str())?,
            kind: match conversation.kind {
                core::ConversationKind::Direct => protocol::ConversationKind::Direct,
                core::ConversationKind::Group(_) => protocol::ConversationKind::Group,
            },
            title: conversation.title.clone(),
            participants: conversation
                .participants
                .iter()
                .map(protocol_participant)
                .collect::<Result<_, _>>()?,
            updated_at: conversation.updated_at.map(protocol_timestamp),
            unread_count,
        })
    }

    fn protocol_message(&self, message: &core::Message) -> Result<Message, ProtocolError> {
        Ok(Message {
            id: id(message.id.as_str())?,
            conversation_id: id(message.conversation_id.as_str())?,
            sender_id: message
                .sender
                .as_ref()
                .map(|value| id(value.as_str()))
                .transpose()?,
            sent_at: protocol_timestamp(message.sent_at),
            parts: message
                .parts
                .iter()
                .map(|part| self.protocol_part(part))
                .collect::<Result<_, _>>()?,
            delivery: protocol_delivery(&message.delivery),
            read: match message.read_state {
                core::ReadState::Unread => ReadState::Unread,
                core::ReadState::Read { .. } => ReadState::Read,
            },
            reactions: message
                .reactions
                .iter()
                .map(protocol_reaction)
                .collect::<Result<_, _>>()?,
        })
    }

    fn protocol_part(&self, part: &core::MessagePart) -> Result<MessagePart, ProtocolError> {
        match part {
            core::MessagePart::Text(value) => Ok(MessagePart {
                kind: MessagePartKind::Text,
                text: Some(protocol_text(value)),
                attachment: None,
                location: None,
                contact: None,
                link_url: None,
                link_title: None,
            }),
            core::MessagePart::Attachment(value) => Ok(MessagePart {
                kind: MessagePartKind::Attachment,
                text: value.caption.as_ref().map(protocol_text),
                attachment: Some(protocol_attachment(&value.attachment, false)?),
                location: None,
                contact: None,
                link_url: None,
                link_title: None,
            }),
            core::MessagePart::LinkPreview(value) => Ok(MessagePart {
                kind: MessagePartKind::Link,
                text: None,
                attachment: value
                    .image
                    .as_ref()
                    .map(|image| protocol_attachment(image, false))
                    .transpose()?,
                location: None,
                contact: None,
                link_url: Some(value.url.clone()),
                link_title: value.title.clone(),
            }),
            core::MessagePart::Location(value) => Ok(MessagePart {
                kind: MessagePartKind::Location,
                text: None,
                attachment: None,
                location: Some(format!(
                    "{},{}",
                    value.point.latitude_e6, value.point.longitude_e6
                )),
                contact: None,
                link_url: None,
                link_title: None,
            }),
            core::MessagePart::Contact(value) => Ok(MessagePart {
                kind: MessagePartKind::Contact,
                text: None,
                attachment: None,
                location: None,
                contact: Some(value.display_name.clone()),
                link_url: None,
                link_title: None,
            }),
            core::MessagePart::ServiceExtension(value) => Ok(MessagePart {
                kind: MessagePartKind::Contact,
                text: None,
                attachment: None,
                location: None,
                contact: Some(format!("{}:{}", value.service, value.name)),
                link_url: None,
                link_title: None,
            }),
        }
    }
}

/// zbus implementation that keeps the wire interface separate from state.
pub struct DbusService {
    service: Arc<dyn DaemonService>,
}

impl DbusService {
    /// Wrap a service implementation for object-server export.
    pub fn new(service: Arc<dyn DaemonService>) -> Self {
        Self { service }
    }

    async fn emit_pending(&self, emitter: &SignalEmitter<'_>) -> Result<(), ProtocolError> {
        for event in self.service.drain_events() {
            Self::event(emitter, event)
                .await
                .map_err(ProtocolError::ZBus)?;
        }
        Ok(())
    }
}

#[zbus::interface(name = "io.github.tannerkrewson.LiteBubbles.Backend.Protocol1")]
impl DbusService {
    #[zbus(name = "Hello")]
    async fn hello(
        &self,
        request: protocol::HelloRequest,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<protocol::HelloResponse, ProtocolError> {
        let result = self.service.hello(request).await;
        self.emit_pending(&emitter).await?;
        result
    }

    #[zbus(name = "GetAccountState")]
    async fn get_account_state(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<AccountState, ProtocolError> {
        let result = self.service.get_account_state().await;
        self.emit_pending(&emitter).await?;
        result
    }

    #[zbus(name = "GetConversations")]
    async fn get_conversations(
        &self,
        request: ConversationPageRequest,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<ConversationPage, ProtocolError> {
        let result = self.service.get_conversations(request).await;
        self.emit_pending(&emitter).await?;
        result
    }

    #[zbus(name = "GetHistory")]
    async fn get_history(
        &self,
        request: HistoryPageRequest,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<HistoryPage, ProtocolError> {
        let result = self.service.get_history(request).await;
        self.emit_pending(&emitter).await?;
        result
    }

    #[zbus(name = "SendMessage")]
    async fn send_message(
        &self,
        request: SendMessageRequest,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<SendMessageResponse, ProtocolError> {
        let result = self.service.send_message(request).await;
        self.emit_pending(&emitter).await?;
        result
    }

    #[zbus(name = "GetAttachment")]
    async fn get_attachment(
        &self,
        attachment_id: Id,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<AttachmentReference, ProtocolError> {
        let result = self.service.get_attachment(attachment_id).await;
        self.emit_pending(&emitter).await?;
        result
    }

    #[zbus(name = "GetSettings")]
    async fn get_settings(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<Settings, ProtocolError> {
        let result = self.service.get_settings().await;
        self.emit_pending(&emitter).await?;
        result
    }

    #[zbus(name = "UpdateSettings")]
    async fn update_settings(
        &self,
        request: UpdateSettingsRequest,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<Settings, ProtocolError> {
        let result = self.service.update_settings(request).await;
        self.emit_pending(&emitter).await?;
        result
    }

    #[zbus(name = "Refresh")]
    async fn refresh(
        &self,
        request: RefreshRequest,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<RefreshResponse, ProtocolError> {
        let result = self.service.refresh(request).await;
        self.emit_pending(&emitter).await?;
        result
    }

    #[zbus(signal, name = "Event")]
    async fn event(signal_emitter: &SignalEmitter<'_>, event: ProtocolEvent) -> zbus::Result<()>;
}

/// Run the daemon until its process is asked to stop by the service manager.
///
/// The connection owns the D-Bus name and object path.  Shutdown is explicit
/// in the guard and is intentionally logged without paths, account values, or
/// backend payloads.
pub async fn run_daemon(database: impl AsRef<Path>) -> Result<(), Box<dyn std::error::Error>> {
    let store = Store::open_path(database).map_err(|error| {
        error!("daemon storage initialization failed");
        Box::<dyn std::error::Error>::from(error)
    })?;
    let service: Arc<dyn DaemonService> = Arc::new(ServiceState::new(store));
    let object = DbusService::new(service);
    let connection = zbus::connection::Builder::session()?
        .name(protocol::BACKEND_BUS_NAME)?
        .serve_at(protocol::OBJECT_PATH, object)?
        .build()
        .await?;
    info!(service = protocol::BACKEND_BUS_NAME, "daemon started");
    let (shutdown_sender, shutdown_receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let Ok(mut signals) = Signals::new([SIGINT, SIGTERM]) else {
            return;
        };
        if signals.forever().next().is_some() {
            let _ = shutdown_sender.try_send(());
        }
    });
    shutdown_receiver.recv().await?;
    drop(connection);
    info!(service = protocol::BACKEND_BUS_NAME, "daemon stopped");
    Ok(())
}

fn storage_error(_error: StorageError) -> ProtocolError {
    error!("local storage operation failed");
    ProtocolError::Internal("local storage operation failed".to_owned())
}

fn adapter_error(error: AdapterError) -> ProtocolError {
    match error {
        AdapterError::Unavailable => ProtocolError::NotReady(RUSTPUSH_BLOCKER.to_owned()),
        AdapterError::Unsupported => {
            ProtocolError::Unsupported("backend operation unavailable".to_owned())
        }
    }
}

fn id(value: &str) -> Result<Id, ProtocolError> {
    Id::new(value.to_owned())
}

fn protocol_timestamp(value: core::Timestamp) -> Timestamp {
    Timestamp(value.as_millis())
}

fn identity(value: &core::Identity) -> Result<protocol::Identity, ProtocolError> {
    Ok(protocol::Identity {
        id: id(value.id.as_str())?,
        display_name: value.label.clone(),
        address: Some(match &value.address {
            core::IdentityAddress::Email(value)
            | core::IdentityAddress::Phone(value)
            | core::IdentityAddress::AppleAccount(value) => value.clone(),
            core::IdentityAddress::Other { value, .. } => value.clone(),
        }),
    })
}

fn device(value: &core::Device) -> Result<Device, ProtocolError> {
    Ok(Device {
        id: id(value.id.as_str())?,
        name: value.name.clone(),
        online: false,
        last_seen_at: value.last_seen_at.map(protocol_timestamp),
    })
}

fn protocol_participant(value: &core::Participant) -> Result<protocol::Participant, ProtocolError> {
    Ok(protocol::Participant {
        id: id(value.id.as_str())?,
        identity_id: value
            .identity_id
            .as_ref()
            .map(|value| id(value.as_str()))
            .transpose()?,
        display_name: value.display_name.clone(),
        role: match value.role {
            core::ParticipantRole::Member => protocol::ParticipantRole::Member,
            core::ParticipantRole::Owner => protocol::ParticipantRole::Owner,
            core::ParticipantRole::Administrator => protocol::ParticipantRole::Administrator,
        },
    })
}

fn protocol_text(value: &core::TextPart) -> protocol::TextPart {
    protocol::TextPart {
        text: value.text.clone(),
        formatting: value
            .formatting
            .iter()
            .map(|formatting| (formatting.range.start, formatting.range.end))
            .collect(),
    }
}

fn protocol_attachment(
    value: &core::Attachment,
    _allow_local_location: bool,
) -> Result<AttachmentReference, ProtocolError> {
    let result = AttachmentReference {
        attachment_id: id(value.id.as_str())?,
        kind: match &value.kind {
            core::AttachmentKind::Image => AttachmentKind::Image,
            core::AttachmentKind::Video => AttachmentKind::Video,
            core::AttachmentKind::Audio => AttachmentKind::Audio,
            core::AttachmentKind::File => AttachmentKind::File,
            core::AttachmentKind::Sticker => AttachmentKind::Sticker,
            core::AttachmentKind::Contact => AttachmentKind::Contact,
            core::AttachmentKind::Location => AttachmentKind::Location,
            core::AttachmentKind::Other(_) => AttachmentKind::Other,
        },
        file_name: value.file_name.clone(),
        mime_type: value.mime_type.clone(),
        byte_size: value.byte_size,
        location: None,
    };
    result.validate()?;
    Ok(result)
}

fn protocol_delivery(value: &core::DeliveryState) -> DeliveryState {
    match value {
        core::DeliveryState::Queued => DeliveryState::Queued,
        core::DeliveryState::Sent => DeliveryState::Sent,
        core::DeliveryState::Delivered => DeliveryState::Delivered,
        core::DeliveryState::Failed { .. } => DeliveryState::Failed,
    }
}

fn protocol_reaction(value: &core::Reaction) -> Result<protocol::Reaction, ProtocolError> {
    Ok(protocol::Reaction {
        id: id(value.id.as_str())?,
        message_id: id(value.message_id.as_str())?,
        participant_id: id(value.participant_id.as_str())?,
        kind: match &value.kind {
            core::ReactionKind::Love => protocol::ReactionKind::Love,
            core::ReactionKind::Like => protocol::ReactionKind::Like,
            core::ReactionKind::Dislike => protocol::ReactionKind::Dislike,
            core::ReactionKind::Laugh => protocol::ReactionKind::Laugh,
            core::ReactionKind::Emphasize => protocol::ReactionKind::Emphasize,
            core::ReactionKind::Question => protocol::ReactionKind::Question,
            core::ReactionKind::Custom(_) => protocol::ReactionKind::Custom,
        },
        removed: value.removed_at.is_some(),
    })
}

fn parse_index_cursor(cursor: Option<&str>) -> Result<usize, ProtocolError> {
    cursor
        .map(|value| {
            value.parse::<usize>().map_err(|_| {
                ProtocolError::InvalidRequest("conversation cursor is invalid".to_owned())
            })
        })
        .transpose()
        .map(|value| value.unwrap_or(0))
}

fn parse_message_cursor(cursor: &str) -> Result<MessageCursor, ProtocolError> {
    let (sent_at, id_value) = cursor
        .split_once(':')
        .ok_or_else(|| ProtocolError::InvalidRequest("history cursor is invalid".to_owned()))?;
    let sent_at = sent_at
        .parse::<i64>()
        .map_err(|_| ProtocolError::InvalidRequest("history cursor is invalid".to_owned()))?;
    let id = core::MessageId::try_from(id_value.to_owned())
        .map_err(|_| ProtocolError::InvalidRequest("history cursor is invalid".to_owned()))?;
    Ok(MessageCursor {
        sent_at: core::Timestamp::from_millis(sent_at),
        id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_lite::future::block_on;
    use litebubbles_mock_backend::MockBackend;

    fn fixture_service() -> ServiceState {
        let fixture = MockBackend::new().fixture().clone();
        let mut store = Store::open_in_memory().expect("in-memory store");
        store.save_account(&fixture.account).expect("account");
        for conversation in &fixture.conversations {
            store.save_conversation(conversation).expect("conversation");
        }
        for message in &fixture.messages {
            store.save_message(message).expect("message");
        }
        ServiceState::with_adapter_and_clock(
            store,
            UnavailableBackend,
            Arc::new(FixedClock(1_700_000_000_000)),
        )
    }

    #[test]
    fn no_credentials_fixture_is_storage_backed_and_bodyless() {
        let service = fixture_service();
        let account = block_on(service.get_account_state()).expect("account state");
        assert_eq!(account.account_id.as_str(), "account-fixture");
        assert_eq!(account.sync.error.as_deref(), Some(RUSTPUSH_BLOCKER));

        let attachment =
            block_on(service.get_attachment(Id::new("attachment-garden-photo").unwrap()))
                .expect("attachment metadata");
        assert!(attachment.location.is_none());
        assert_eq!(attachment.byte_size, Some(12));
    }

    #[test]
    fn service_paginates_deterministically_and_negotiates_versions() {
        let service = fixture_service();
        let hello =
            block_on(service.hello(protocol::HelloRequest::new("test", 1, 1))).expect("hello");
        assert_eq!(hello.version, protocol::PROTOCOL_VERSION);
        let first =
            block_on(service.get_conversations(ConversationPageRequest::new(None, 1).unwrap()))
                .expect("first page");
        assert_eq!(first.conversations.len(), 1);
        let second = block_on(
            service.get_conversations(ConversationPageRequest::new(first.next_cursor, 1).unwrap()),
        )
        .expect("second page");
        assert_eq!(second.conversations.len(), 1);
        assert_ne!(first.conversations[0].id, second.conversations[0].id);
    }

    #[test]
    fn unavailable_refresh_is_explicit_and_emits_typed_event() {
        let service = fixture_service();
        let response = block_on(service.refresh(RefreshRequest {
            scope: protocol::RefreshScope::All,
            account_id: None,
        }))
        .expect("refresh response");
        assert_eq!(response.state, RefreshState::Failed);
        let events = service.drain_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, EventKind::SyncChanged);
        assert_eq!(
            events[0].sync.as_ref().unwrap().state.error.as_deref(),
            Some(RUSTPUSH_BLOCKER)
        );
    }
}
