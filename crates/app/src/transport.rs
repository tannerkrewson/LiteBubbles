//! Application-side transport boundary for the LiteBubbles daemon.
//!
//! This module is the only app layer that knows about zbus or the wire
//! protocol. Views and view models can depend on [`AppTransport`] without
//! knowing how a daemon connection is created, negotiated, retried, or
//! reported when it fails. The GTK shell chooses [`ShellTransportMode::Dbus`]
//! by default and keeps the existing [`litebubbles_mock_backend`] fixture
//! behind an explicit development switch.

use std::{
    fmt,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use litebubbles_protocol as protocol;
use litebubbles_protocol::{
    AccountState, ConversationPage, ConversationPageRequest, HelloRequest, HelloResponse,
    HistoryPage, HistoryPageRequest, ProtocolError, RefreshRequest, RefreshResponse,
    SendMessageRequest, SendMessageResponse, Settings, UpdateSettingsRequest,
};

/// Stable client name sent during protocol negotiation.
pub const CLIENT_NAME: &str = "litebubbles";

/// Capabilities understood by this transport boundary.
pub const CLIENT_CAPABILITIES: &[&str] = &["events"];

/// Environment variable used to select the shell transport.
pub const TRANSPORT_ENVIRONMENT_VARIABLE: &str = "LITEBUBBLES_TRANSPORT";

/// Exact value that selects the deterministic fixture transport.
pub const MOCK_TRANSPORT_VALUE: &str = "mock";

/// Transport selected by the GTK shell.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellTransportMode {
    /// Use the user's session D-Bus and the daemon protocol.
    Dbus,
    /// Use the in-process synthetic fixture for development and tests.
    Mock,
}

impl ShellTransportMode {
    /// Selects the production transport unless the explicit mock switch is set.
    pub fn from_environment() -> Self {
        Self::from_value(
            std::env::var(TRANSPORT_ENVIRONMENT_VARIABLE)
                .ok()
                .as_deref(),
        )
    }

    /// Selects a mode from an environment value without reading process state.
    pub fn from_value(value: Option<&str>) -> Self {
        match value {
            Some(MOCK_TRANSPORT_VALUE) => Self::Mock,
            _ => Self::Dbus,
        }
    }

    /// Returns whether this mode owns the synthetic fixture shell.
    pub const fn uses_fixture(self) -> bool {
        matches!(self, Self::Mock)
    }
}

/// A successfully negotiated daemon protocol.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NegotiatedProtocol {
    version: u16,
    server_name: String,
    capabilities: Vec<String>,
}

impl NegotiatedProtocol {
    /// Returns the selected protocol version.
    pub const fn version(&self) -> u16 {
        self.version
    }

    /// Returns the daemon's diagnostic name.
    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// Returns the capabilities advertised by the daemon.
    pub fn capabilities(&self) -> &[String] {
        &self.capabilities
    }

    fn from_response(response: HelloResponse) -> Result<Self, TransportError> {
        if !(protocol::MIN_SUPPORTED_VERSION..=protocol::MAX_SUPPORTED_VERSION)
            .contains(&response.version)
        {
            return Err(TransportError::ProtocolMismatch);
        }
        Ok(Self {
            version: response.version,
            server_name: response.server_name,
            capabilities: response.capabilities,
        })
    }
}

/// Safe, app-facing transport failures.
///
/// The variants intentionally contain no D-Bus error text, account values, or
/// message content. Protocol error details are daemon-facing diagnostics and
/// must not become widget text or accidental logs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportError {
    /// The daemon connection could not be created or was lost.
    Unavailable,
    /// An operation was attempted before [`AppTransport::negotiate`].
    NotNegotiated,
    /// The daemon and app do not share a supported protocol version.
    ProtocolMismatch,
    /// The request failed app/protocol validation.
    InvalidRequest,
    /// The requested account, conversation, message, or attachment is absent.
    NotFound,
    /// The daemon rejected the operation for the current session.
    PermissionDenied,
    /// The daemon is not ready for the operation.
    NotReady,
    /// The request conflicts with newer state.
    Conflict,
    /// The requested operation is not available.
    Unsupported,
    /// The daemon could not complete the operation.
    Failed,
}

impl TransportError {
    /// Returns whether reconnecting or resynchronizing may make a retry useful.
    pub const fn is_retryable(self) -> bool {
        matches!(
            self,
            Self::Unavailable | Self::NotNegotiated | Self::NotReady | Self::Failed
        )
    }

    /// Converts any handshake failure into the shell's redacted recoverable
    /// status. The specific protocol or D-Bus detail never crosses into the UI.
    pub const fn shell_status(self) -> ShellTransportStatus {
        ShellTransportStatus::Recoverable
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Unavailable => "the LiteBubbles service is unavailable",
            Self::NotNegotiated => "the LiteBubbles service has not been negotiated",
            Self::ProtocolMismatch => "the LiteBubbles service uses an unsupported protocol",
            Self::InvalidRequest => "the request was not accepted",
            Self::NotFound => "the requested item was not found",
            Self::PermissionDenied => "the request is not permitted",
            Self::NotReady => "the service is not ready",
            Self::Conflict => "the request conflicts with newer state",
            Self::Unsupported => "the operation is not supported",
            Self::Failed => "the service could not complete the request",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for TransportError {}

/// Safe status values exposed by the shell transport seam.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellTransportStatus {
    /// A connection and protocol negotiation are in flight.
    Connecting,
    /// The daemon connection has completed the protocol handshake.
    Connected,
    /// The handshake failed and can be retried without exposing details.
    Recoverable,
}

impl ShellTransportStatus {
    /// Maps a transport result to a status without retaining its error detail.
    pub fn from_transport_result<T>(result: Result<T, TransportError>) -> Self {
        match result {
            Ok(_) => Self::Connected,
            Err(error) => error.shell_status(),
        }
    }

    /// Returns whether the shell should offer retry.
    pub const fn is_recoverable(self) -> bool {
        matches!(self, Self::Recoverable)
    }

    /// Returns a safe title for a shell status page.
    pub const fn title(self) -> &'static str {
        match self {
            Self::Connecting => "Connecting to LiteBubbles",
            Self::Connected => "Connected to LiteBubbles",
            Self::Recoverable => "LiteBubbles service unavailable",
        }
    }

    /// Returns a safe description for a shell status page.
    pub const fn description(self) -> &'static str {
        match self {
            Self::Connecting => "Preparing the daemon connection…",
            Self::Connected => "The daemon handshake completed.",
            Self::Recoverable => "The service could not be reached. You can retry.",
        }
    }
}

impl From<ProtocolError> for TransportError {
    fn from(error: ProtocolError) -> Self {
        match error {
            ProtocolError::UnsupportedVersion(_) => Self::ProtocolMismatch,
            ProtocolError::InvalidRequest(_) => Self::InvalidRequest,
            ProtocolError::NotFound(_) => Self::NotFound,
            ProtocolError::PermissionDenied(_) => Self::PermissionDenied,
            ProtocolError::NotReady(_) => Self::NotReady,
            ProtocolError::Conflict(_) => Self::Conflict,
            ProtocolError::Unsupported(_) => Self::Unsupported,
            ProtocolError::Internal(_) => Self::Failed,
            ProtocolError::ZBus(_) => Self::Unavailable,
        }
    }
}

impl From<zbus::Error> for TransportError {
    fn from(_error: zbus::Error) -> Self {
        Self::Unavailable
    }
}

/// Backend operations exposed to app controllers and view models.
///
/// Implementations may use the daemon, a deterministic fixture, or a future
/// reconnecting/resynchronizing transport. The GTK shell does not need to
/// change when that implementation changes.
#[async_trait]
pub trait AppTransport: Send + Sync {
    /// Negotiate the protocol before any other operation is used.
    async fn negotiate(&self) -> Result<NegotiatedProtocol, TransportError>;

    /// Return account and synchronization state.
    async fn get_account_state(&self) -> Result<AccountState, TransportError>;

    /// Return one page of conversations.
    async fn get_conversations(
        &self,
        request: ConversationPageRequest,
    ) -> Result<ConversationPage, TransportError>;

    /// Return one page of history for a conversation.
    async fn get_history(&self, request: HistoryPageRequest)
    -> Result<HistoryPage, TransportError>;

    /// Return current app settings.
    async fn get_settings(&self) -> Result<Settings, TransportError>;

    /// Apply a partial settings update.
    async fn update_settings(
        &self,
        request: UpdateSettingsRequest,
    ) -> Result<Settings, TransportError>;

    /// Queue an asynchronous account/state refresh.
    async fn refresh(&self, request: RefreshRequest) -> Result<RefreshResponse, TransportError>;

    /// Submit one outgoing message.
    async fn send_message(
        &self,
        request: SendMessageRequest,
    ) -> Result<SendMessageResponse, TransportError>;
}

/// Typed app-side client for the versioned daemon D-Bus service.
#[derive(Clone, Debug)]
pub struct DbusTransport {
    proxy: protocol::LiteBubblesProxy<'static>,
    negotiated_version: Arc<Mutex<Option<u16>>>,
}

impl DbusTransport {
    /// Connect to the user's session bus and negotiate protocol version 1.
    pub async fn connect() -> Result<Self, TransportError> {
        let connection = zbus::Connection::session()
            .await
            .map_err(TransportError::from)?;
        let transport = Self::from_connection(connection).await?;
        transport.negotiate().await?;
        Ok(transport)
    }

    /// Build a client around an already-established D-Bus connection.
    ///
    /// This is useful for credential-free peer tests and for callers that own
    /// bus setup. The returned transport is not negotiated until
    /// [`Self::negotiate`] succeeds.
    pub async fn from_connection(connection: zbus::Connection) -> Result<Self, TransportError> {
        let proxy = protocol::LiteBubblesProxy::new(&connection)
            .await
            .map_err(TransportError::from)?;
        Ok(Self {
            proxy,
            negotiated_version: Arc::new(Mutex::new(None)),
        })
    }

    /// Returns whether this client has completed protocol negotiation.
    pub fn is_negotiated(&self) -> bool {
        self.negotiated_version
            .lock()
            .ok()
            .and_then(|version| *version)
            .is_some()
    }

    /// Negotiate the highest protocol version supported by both peers.
    pub async fn negotiate(&self) -> Result<NegotiatedProtocol, TransportError> {
        self.clear_negotiated();
        let response = self
            .proxy
            .hello(hello_request())
            .await
            .map_err(TransportError::from)?;
        let negotiated = NegotiatedProtocol::from_response(response)?;
        let mut version = self
            .negotiated_version
            .lock()
            .map_err(|_| TransportError::Unavailable)?;
        *version = Some(negotiated.version());
        Ok(negotiated)
    }

    /// Return account and synchronization state.
    pub async fn get_account_state(&self) -> Result<AccountState, TransportError> {
        self.ensure_negotiated()?;
        self.proxy
            .get_account_state()
            .await
            .map_err(TransportError::from)
    }

    /// Return one page of conversations.
    pub async fn get_conversations(
        &self,
        request: ConversationPageRequest,
    ) -> Result<ConversationPage, TransportError> {
        self.ensure_negotiated()?;
        self.proxy
            .get_conversations(request)
            .await
            .map_err(TransportError::from)
    }

    /// Return one page of history for a conversation.
    pub async fn get_history(
        &self,
        request: HistoryPageRequest,
    ) -> Result<HistoryPage, TransportError> {
        self.ensure_negotiated()?;
        self.proxy
            .get_history(request)
            .await
            .map_err(TransportError::from)
    }

    /// Return current app settings.
    pub async fn get_settings(&self) -> Result<Settings, TransportError> {
        self.ensure_negotiated()?;
        self.proxy
            .get_settings()
            .await
            .map_err(TransportError::from)
    }

    /// Apply a partial settings update.
    pub async fn update_settings(
        &self,
        request: UpdateSettingsRequest,
    ) -> Result<Settings, TransportError> {
        self.ensure_negotiated()?;
        self.proxy
            .update_settings(request)
            .await
            .map_err(TransportError::from)
    }

    /// Queue an asynchronous account/state refresh.
    pub async fn refresh(
        &self,
        request: RefreshRequest,
    ) -> Result<RefreshResponse, TransportError> {
        self.ensure_negotiated()?;
        self.proxy
            .refresh(request)
            .await
            .map_err(TransportError::from)
    }

    /// Submit one outgoing message.
    pub async fn send_message(
        &self,
        request: SendMessageRequest,
    ) -> Result<SendMessageResponse, TransportError> {
        self.ensure_negotiated()?;
        self.proxy
            .send_message(request)
            .await
            .map_err(TransportError::from)
    }

    fn ensure_negotiated(&self) -> Result<(), TransportError> {
        match self.negotiated_version.lock() {
            Ok(version) if version.is_some() => Ok(()),
            Ok(_) | Err(_) => Err(TransportError::NotNegotiated),
        }
    }

    fn clear_negotiated(&self) {
        if let Ok(mut version) = self.negotiated_version.lock() {
            *version = None;
        }
    }
}

#[async_trait]
impl AppTransport for DbusTransport {
    async fn negotiate(&self) -> Result<NegotiatedProtocol, TransportError> {
        Self::negotiate(self).await
    }

    async fn get_account_state(&self) -> Result<AccountState, TransportError> {
        Self::get_account_state(self).await
    }

    async fn get_conversations(
        &self,
        request: ConversationPageRequest,
    ) -> Result<ConversationPage, TransportError> {
        Self::get_conversations(self, request).await
    }

    async fn get_history(
        &self,
        request: HistoryPageRequest,
    ) -> Result<HistoryPage, TransportError> {
        Self::get_history(self, request).await
    }

    async fn get_settings(&self) -> Result<Settings, TransportError> {
        Self::get_settings(self).await
    }

    async fn update_settings(
        &self,
        request: UpdateSettingsRequest,
    ) -> Result<Settings, TransportError> {
        Self::update_settings(self, request).await
    }

    async fn refresh(&self, request: RefreshRequest) -> Result<RefreshResponse, TransportError> {
        Self::refresh(self, request).await
    }

    async fn send_message(
        &self,
        request: SendMessageRequest,
    ) -> Result<SendMessageResponse, TransportError> {
        Self::send_message(self, request).await
    }
}

fn hello_request() -> HelloRequest {
    let mut request = HelloRequest::new(
        CLIENT_NAME,
        protocol::MIN_SUPPORTED_VERSION,
        protocol::MAX_SUPPORTED_VERSION,
    );
    request.capabilities = CLIENT_CAPABILITIES
        .iter()
        .map(|capability| (*capability).to_owned())
        .collect();
    request
}

#[cfg(test)]
mod tests {
    use super::*;
    use litebubbles_protocol::{
        Conversation, ConversationKind, DeliveryState, Device, Id, Identity, Message, MessagePart,
        MessagePartKind, NotificationMode, Participant, ParticipantRole, ReadState, RefreshScope,
        RefreshState, SyncState, TextPart,
    };

    fn fixture_id(value: &str) -> Id {
        Id::new(value).expect("fixture identifier is valid")
    }

    #[test]
    fn hello_request_uses_the_supported_range_and_safe_capabilities() {
        let request = hello_request();

        assert_eq!(request.client_name, CLIENT_NAME);
        assert_eq!(request.min_version, protocol::MIN_SUPPORTED_VERSION);
        assert_eq!(request.max_version, protocol::MAX_SUPPORTED_VERSION);
        assert_eq!(request.capabilities, vec!["events"]);
    }

    #[test]
    fn shell_transport_defaults_to_dbus_and_only_explicit_mock_selects_fixture() {
        assert_eq!(
            ShellTransportMode::from_value(None),
            ShellTransportMode::Dbus
        );
        assert_eq!(
            ShellTransportMode::from_value(Some(MOCK_TRANSPORT_VALUE)),
            ShellTransportMode::Mock
        );
        assert_eq!(
            ShellTransportMode::from_value(Some("MOCK")),
            ShellTransportMode::Dbus
        );
        assert_eq!(
            ShellTransportMode::from_value(Some("fixture")),
            ShellTransportMode::Dbus
        );
    }

    #[test]
    fn shell_transport_failures_map_to_one_redacted_recoverable_status() {
        let failures = [
            TransportError::Unavailable,
            TransportError::NotNegotiated,
            TransportError::ProtocolMismatch,
            TransportError::InvalidRequest,
            TransportError::NotFound,
            TransportError::PermissionDenied,
            TransportError::NotReady,
            TransportError::Conflict,
            TransportError::Unsupported,
            TransportError::Failed,
        ];

        for error in failures {
            assert_eq!(error.shell_status(), ShellTransportStatus::Recoverable);
            assert!(error.shell_status().is_recoverable());
            assert!(!error.shell_status().description().contains("D-Bus"));
        }

        assert_eq!(
            ShellTransportStatus::from_transport_result(Result::<(), _>::Ok(())),
            ShellTransportStatus::Connected
        );
        assert_eq!(
            ShellTransportStatus::from_transport_result(Result::<(), _>::Err(
                TransportError::Unavailable,
            )),
            ShellTransportStatus::Recoverable
        );
        assert_eq!(
            ShellTransportStatus::Recoverable.description(),
            "The service could not be reached. You can retry."
        );
    }

    #[test]
    fn transport_errors_drop_protocol_and_transport_details() {
        let detail = "message body that must not reach the app";
        let mappings = [
            (
                ProtocolError::UnsupportedVersion(detail.to_owned()),
                TransportError::ProtocolMismatch,
            ),
            (
                ProtocolError::InvalidRequest(detail.to_owned()),
                TransportError::InvalidRequest,
            ),
            (
                ProtocolError::NotFound(detail.to_owned()),
                TransportError::NotFound,
            ),
            (
                ProtocolError::PermissionDenied(detail.to_owned()),
                TransportError::PermissionDenied,
            ),
            (
                ProtocolError::NotReady(detail.to_owned()),
                TransportError::NotReady,
            ),
            (
                ProtocolError::Conflict(detail.to_owned()),
                TransportError::Conflict,
            ),
            (
                ProtocolError::Unsupported(detail.to_owned()),
                TransportError::Unsupported,
            ),
            (
                ProtocolError::Internal(detail.to_owned()),
                TransportError::Failed,
            ),
            (
                ProtocolError::ZBus(zbus::Error::Failure(detail.to_owned())),
                TransportError::Unavailable,
            ),
        ];

        for (error, expected) in mappings {
            let mapped = TransportError::from(error);
            assert_eq!(mapped, expected);
            assert!(!mapped.to_string().contains(detail));
        }

        let mapped = TransportError::from(zbus::Error::Failure(detail.to_owned()));
        assert_eq!(mapped, TransportError::Unavailable);
        assert!(!mapped.to_string().contains(detail));
    }

    #[test]
    fn invalid_hello_response_is_not_accepted() {
        let response = HelloResponse {
            version: protocol::MAX_SUPPORTED_VERSION.saturating_add(1),
            server_name: "fixture".to_owned(),
            capabilities: Vec::new(),
        };

        assert_eq!(
            NegotiatedProtocol::from_response(response),
            Err(TransportError::ProtocolMismatch)
        );
    }

    #[cfg(unix)]
    #[test]
    fn peer_client_negotiates_and_round_trips_representative_requests() {
        zbus::block_on(async {
            let (_server, client) = fixture_transport().await;

            assert!(!client.is_negotiated());
            assert_eq!(
                client.get_settings().await,
                Err(TransportError::NotNegotiated)
            );

            let negotiated = client.negotiate().await.expect("fixture negotiation");
            assert_eq!(negotiated.version(), protocol::PROTOCOL_VERSION);
            assert_eq!(negotiated.server_name(), "fixture-daemon");
            assert_eq!(negotiated.capabilities(), &["events".to_owned()]);
            assert!(client.is_negotiated());

            let account = client
                .get_account_state()
                .await
                .expect("fixture account state");
            assert_eq!(account.account_id.as_str(), "account-fixture");
            assert!(account.sync.connected);

            let conversations = client
                .get_conversations(ConversationPageRequest::new(None, 10).unwrap())
                .await
                .expect("fixture conversations");
            assert_eq!(conversations.conversations.len(), 1);
            assert_eq!(
                conversations.conversations[0].id.as_str(),
                "conversation-fixture"
            );

            let history = client
                .get_history(
                    HistoryPageRequest::new(fixture_id("conversation-fixture"), None, 10).unwrap(),
                )
                .await
                .expect("fixture history");
            assert_eq!(history.messages.len(), 1);
            assert_eq!(history.messages[0].id.as_str(), "message-fixture");

            let settings = client.get_settings().await.expect("fixture settings");
            assert_eq!(settings.notifications, NotificationMode::All);
            let settings = client
                .update_settings(UpdateSettingsRequest {
                    notifications: Some(NotificationMode::Mentions),
                    show_previews: Some(false),
                    allow_local_attachment_locations: None,
                })
                .await
                .expect("fixture settings update");
            assert_eq!(settings.notifications, NotificationMode::Mentions);
            assert!(!settings.show_previews);

            let refresh = client
                .refresh(RefreshRequest {
                    scope: RefreshScope::All,
                    account_id: None,
                })
                .await
                .expect("fixture refresh");
            assert_eq!(refresh.state, RefreshState::Queued);

            let send = client
                .send_message(SendMessageRequest {
                    conversation_id: fixture_id("conversation-fixture"),
                    parts: vec![litebubbles_protocol::OutgoingPart {
                        kind: MessagePartKind::Text,
                        text: Some(TextPart {
                            text: "fixture message".to_owned(),
                            formatting: Vec::new(),
                        }),
                        attachment: None,
                    }],
                    client_mutation_id: fixture_id("mutation-fixture"),
                })
                .await
                .expect("fixture send");
            assert_eq!(send.message_id.as_str(), "message-fixture");
            assert_eq!(send.delivery, DeliveryState::Queued);
        });
    }

    #[cfg(unix)]
    async fn fixture_transport() -> (zbus::Connection, DbusTransport) {
        use std::os::unix::net::UnixStream;

        let (server_socket, client_socket) = UnixStream::pair().expect("peer sockets");
        let guid = zbus::Guid::generate();
        let server_builder = zbus::connection::Builder::unix_stream(server_socket)
            .server(guid)
            .expect("server GUID")
            .p2p()
            .serve_at(protocol::OBJECT_PATH, FixtureService::default())
            .expect("fixture service");
        let client_builder = zbus::connection::Builder::unix_stream(client_socket).p2p();
        let (server, client) =
            futures_lite::future::zip(server_builder.build(), client_builder.build()).await;
        let server = server.expect("fixture server connection");
        let client = client.expect("fixture client connection");
        let transport = DbusTransport::from_connection(client)
            .await
            .expect("fixture client transport");
        (server, transport)
    }

    #[cfg(unix)]
    #[derive(Debug)]
    struct FixtureService {
        settings: Mutex<Settings>,
    }

    #[cfg(unix)]
    impl FixtureService {
        fn with_settings() -> Self {
            Self {
                settings: Mutex::new(Settings {
                    notifications: NotificationMode::All,
                    show_previews: true,
                    allow_local_attachment_locations: false,
                }),
            }
        }
    }

    #[cfg(unix)]
    impl Default for FixtureService {
        fn default() -> Self {
            Self::with_settings()
        }
    }

    #[cfg(unix)]
    #[zbus::interface(name = "io.github.tannerkrewson.LiteBubbles.Backend.Protocol1")]
    impl FixtureService {
        #[zbus(name = "Hello")]
        fn hello(&self, request: HelloRequest) -> Result<HelloResponse, ProtocolError> {
            Ok(HelloResponse {
                version: protocol::negotiate_version(&request)?,
                server_name: "fixture-daemon".to_owned(),
                capabilities: vec!["events".to_owned()],
            })
        }

        #[zbus(name = "GetAccountState")]
        fn get_account_state(&self) -> Result<AccountState, ProtocolError> {
            Ok(AccountState {
                account_id: fixture_id("account-fixture"),
                display_name: Some("Fixture Account".to_owned()),
                identities: vec![Identity {
                    id: fixture_id("identity-fixture"),
                    display_name: Some("Fixture Identity".to_owned()),
                    address: Some("fixture@example.invalid".to_owned()),
                }],
                devices: vec![Device {
                    id: fixture_id("device-fixture"),
                    name: Some("Fixture Device".to_owned()),
                    online: true,
                    last_seen_at: Some(protocol::Timestamp(100)),
                }],
                sync: SyncState {
                    connected: true,
                    initial_sync_complete: true,
                    last_success_at: Some(protocol::Timestamp(100)),
                    error: None,
                },
            })
        }

        #[zbus(name = "GetConversations")]
        fn get_conversations(
            &self,
            request: ConversationPageRequest,
        ) -> Result<ConversationPage, ProtocolError> {
            ConversationPageRequest::new(request.cursor, request.limit)?;
            Ok(ConversationPage {
                conversations: vec![Conversation {
                    id: fixture_id("conversation-fixture"),
                    kind: ConversationKind::Direct,
                    title: Some("Fixture Conversation".to_owned()),
                    participants: vec![Participant {
                        id: fixture_id("participant-fixture"),
                        identity_id: Some(fixture_id("identity-fixture")),
                        display_name: Some("Fixture Person".to_owned()),
                        role: ParticipantRole::Member,
                    }],
                    updated_at: Some(protocol::Timestamp(100)),
                    unread_count: 1,
                }],
                next_cursor: None,
            })
        }

        #[zbus(name = "GetHistory")]
        fn get_history(&self, request: HistoryPageRequest) -> Result<HistoryPage, ProtocolError> {
            HistoryPageRequest::new(
                request.conversation_id.clone(),
                request.cursor,
                request.limit,
            )?;
            if request.conversation_id.as_str() != "conversation-fixture" {
                return Err(ProtocolError::NotFound("fixture conversation".to_owned()));
            }
            Ok(HistoryPage {
                messages: vec![Message {
                    id: fixture_id("message-fixture"),
                    conversation_id: fixture_id("conversation-fixture"),
                    sender_id: Some(fixture_id("participant-fixture")),
                    sent_at: protocol::Timestamp(100),
                    parts: vec![MessagePart {
                        kind: MessagePartKind::Text,
                        text: Some(TextPart {
                            text: "fixture history".to_owned(),
                            formatting: Vec::new(),
                        }),
                        attachment: None,
                        location: None,
                        contact: None,
                        link_url: None,
                        link_title: None,
                    }],
                    delivery: DeliveryState::Delivered,
                    read: ReadState::Read,
                    reactions: Vec::new(),
                }],
                next_cursor: None,
            })
        }

        #[zbus(name = "SendMessage")]
        fn send_message(
            &self,
            request: SendMessageRequest,
        ) -> Result<SendMessageResponse, ProtocolError> {
            if request.parts.is_empty() {
                return Err(ProtocolError::InvalidRequest("message is empty".to_owned()));
            }
            Ok(SendMessageResponse {
                message_id: fixture_id("message-fixture"),
                delivery: DeliveryState::Queued,
                accepted_at: protocol::Timestamp(100),
            })
        }

        #[zbus(name = "GetSettings")]
        fn get_settings(&self) -> Result<Settings, ProtocolError> {
            self.settings
                .lock()
                .map(|settings| settings.clone())
                .map_err(|_| ProtocolError::Internal("fixture settings lock".to_owned()))
        }

        #[zbus(name = "UpdateSettings")]
        fn update_settings(
            &self,
            request: UpdateSettingsRequest,
        ) -> Result<Settings, ProtocolError> {
            let mut settings = self
                .settings
                .lock()
                .map_err(|_| ProtocolError::Internal("fixture settings lock".to_owned()))?;
            if let Some(notifications) = request.notifications {
                settings.notifications = notifications;
            }
            if let Some(show_previews) = request.show_previews {
                settings.show_previews = show_previews;
            }
            if let Some(allow_local_attachment_locations) = request.allow_local_attachment_locations
            {
                settings.allow_local_attachment_locations = allow_local_attachment_locations;
            }
            Ok(settings.clone())
        }

        #[zbus(name = "Refresh")]
        fn refresh(&self, _request: RefreshRequest) -> Result<RefreshResponse, ProtocolError> {
            Ok(RefreshResponse {
                operation_id: fixture_id("refresh-fixture"),
                state: RefreshState::Queued,
            })
        }
    }

    #[cfg(unix)]
    #[test]
    fn fixture_service_starts_with_default_settings() {
        let service = FixtureService::with_settings();
        let settings = service.settings.lock().expect("fixture settings lock");
        assert_eq!(settings.notifications, NotificationMode::All);
    }
}
