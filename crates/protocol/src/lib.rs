//! Versioned D-Bus contract between `litebubblesd` and its clients.
//!
//! The types in this crate are wire types. They intentionally do not expose
//! backend or `litebubbles-core` implementation types, so a daemon can map a
//! backend into this contract without making that backend part of the client
//! ABI. Attachment bodies never cross this interface: messages contain stable
//! attachment IDs and optional, daemon-controlled local locations only.

use std::{fmt, path::Path};

use serde::{Deserialize, Serialize};
use zbus::DBusError;
use zvariant::Type;

/// The major version encoded in the D-Bus interface name.
pub const PROTOCOL_VERSION: u16 = 1;
/// The lowest protocol version implemented by this crate.
pub const MIN_SUPPORTED_VERSION: u16 = PROTOCOL_VERSION;
/// The highest protocol version implemented by this crate.
pub const MAX_SUPPORTED_VERSION: u16 = PROTOCOL_VERSION;
/// GApplication identity derived from the authenticated GitHub owner.
pub const APPLICATION_ID: &str = "io.github.tannerkrewson.LiteBubbles";
/// Well-known service name owned by the long-running backend daemon.
pub const BACKEND_BUS_NAME: &str = "io.github.tannerkrewson.LiteBubbles.Backend";
/// Backward-compatible alias for the daemon's well-known service name.
pub const BUS_NAME: &str = BACKEND_BUS_NAME;
/// Object path exported by the backend daemon.
pub const OBJECT_PATH: &str = "/io/github/tannerkrewson/LiteBubbles/Backend";
/// Stable D-Bus interface name for protocol version 1.
pub const INTERFACE_NAME: &str = "io.github.tannerkrewson.LiteBubbles.Backend.Protocol1";
/// Maximum number of records returned by one paginated request.
pub const MAX_PAGE_SIZE: u32 = 500;

/// A hello sent before any versioned operation is used.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct HelloRequest {
    /// Client-defined name used for diagnostics.
    pub client_name: String,
    /// Lowest protocol version the client can use.
    pub min_version: u16,
    /// Highest protocol version the client can use.
    pub max_version: u16,
    /// Optional capability names understood by the client.
    pub capabilities: Vec<String>,
}

impl HelloRequest {
    /// Creates a hello for a client supporting an inclusive version range.
    pub fn new(client_name: impl Into<String>, min_version: u16, max_version: u16) -> Self {
        Self {
            client_name: client_name.into(),
            min_version,
            max_version,
            capabilities: Vec::new(),
        }
    }

    /// Returns whether the request's version range is well-formed.
    pub const fn is_valid(&self) -> bool {
        self.min_version != 0 && self.min_version <= self.max_version
    }
}

/// The daemon's response to [`HelloRequest`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct HelloResponse {
    /// The version selected for this connection.
    pub version: u16,
    /// Daemon-defined name used for diagnostics.
    pub server_name: String,
    /// Capabilities available at the selected version.
    pub capabilities: Vec<String>,
}

/// Selects the highest mutually supported version, if one exists.
pub fn negotiate_version(request: &HelloRequest) -> Result<u16, ProtocolError> {
    if !request.is_valid() {
        return Err(ProtocolError::InvalidRequest(
            "hello version range is invalid".to_owned(),
        ));
    }
    let version = request.max_version.min(MAX_SUPPORTED_VERSION);
    if version < request.min_version || version < MIN_SUPPORTED_VERSION {
        return Err(ProtocolError::UnsupportedVersion(format!(
            "client supports {}..={}, server supports {}..={}",
            request.min_version, request.max_version, MIN_SUPPORTED_VERSION, MAX_SUPPORTED_VERSION
        )));
    }
    Ok(version)
}

/// Typed failures returned by protocol methods.
#[derive(Debug, DBusError)]
#[zbus(
    prefix = "io.github.tannerkrewson.LiteBubbles.Backend.Protocol1.Error",
    impl_display = true
)]
pub enum ProtocolError {
    /// The client and daemon have no mutually supported protocol version.
    UnsupportedVersion(String),
    /// A request failed local protocol validation.
    InvalidRequest(String),
    /// A requested account, conversation, message, or attachment was absent.
    NotFound(String),
    /// The operation is not permitted for the current account/session.
    PermissionDenied(String),
    /// The daemon has not completed the required account or sync setup.
    NotReady(String),
    /// The request conflicts with newer state.
    Conflict(String),
    /// The requested feature is not available from the backend.
    Unsupported(String),
    /// A daemon-side failure that is safe to show to the client.
    Internal(String),
    /// A transport failure while dispatching a method.
    #[zbus(error)]
    ZBus(zbus::Error),
}

/// An opaque, non-empty identifier owned by the daemon or backend.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Type)]
pub struct Id(pub String);

impl Id {
    /// Creates an identifier, rejecting empty and whitespace-only values.
    pub fn new(value: impl Into<String>) -> Result<Self, ProtocolError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(ProtocolError::InvalidRequest(
                "identifiers must not be empty".to_owned(),
            ));
        }
        Ok(Self(value))
    }

    /// Borrows the identifier's stable wire value.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Id {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

macro_rules! string_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$variant_meta:meta])* $variant:ident),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
        #[zvariant(signature = "s", rename_all = "snake_case")]
        pub enum $name {
            $($(#[$variant_meta])* $variant),+
        }
    };
}

string_enum! {
    /// Kind of a conversation.
    ConversationKind { Direct, Group }
}
string_enum! {
    /// Role held by a conversation participant.
    ParticipantRole { Member, Owner, Administrator }
}
string_enum! {
    /// Kind of message content.
    MessagePartKind { Text, Attachment, Location, Contact, Link }
}
string_enum! {
    /// Media category for an attachment reference.
    AttachmentKind { Image, Video, Audio, File, Sticker, Contact, Location, Other }
}
string_enum! {
    /// Delivery state of an outgoing message.
    DeliveryState { Queued, Sent, Delivered, Failed }
}
string_enum! {
    /// Typing state observed in a conversation.
    TypingState { Started, Stopped }
}
string_enum! {
    /// Kind of reaction attached to a message.
    ReactionKind { Love, Like, Dislike, Laugh, Emphasize, Question, Custom }
}
string_enum! {
    /// State of a read marker.
    ReadState { Unread, Read }
}
string_enum! {
    /// Type of call represented by a call event.
    CallKind { Audio, Video }
}
string_enum! {
    /// Lifecycle state of a call.
    CallState { Ringing, Active, Held, Ended, Failed }
}
string_enum! {
    /// Availability state of a Find My item.
    FindMyStatus { Online, Offline, Lost, Unknown }
}
string_enum! {
    /// Scope of a daemon refresh request.
    RefreshScope { Account, Conversations, History, Attachments, All }
}
string_enum! {
    /// Lifecycle state of a refresh operation.
    RefreshState { Queued, Running, Complete, Failed }
}
string_enum! {
    /// Setting controlling whether notifications are shown.
    NotificationMode { All, Mentions, None }
}
string_enum! {
    /// Variant discriminator for [`ProtocolEvent`].
    EventKind {
        AccountChanged,
        ConversationChanged,
        MessageChanged,
        ReactionChanged,
        TypingChanged,
        ReadChanged,
        SyncChanged,
        AttachmentChanged,
        CallChanged,
        FindMyChanged
    }
}

/// Milliseconds since the Unix epoch.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize, Type,
)]
pub struct Timestamp(pub i64);

/// A person/identity shown in account or conversation state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct Identity {
    /// Stable identity identifier.
    pub id: Id,
    /// Display name, if known.
    pub display_name: Option<String>,
    /// Backend-normalized address, if known.
    pub address: Option<String>,
}

/// A device associated with an account.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct Device {
    /// Stable device identifier.
    pub id: Id,
    /// User-facing device name.
    pub name: Option<String>,
    /// Whether the device is currently reachable.
    pub online: bool,
    /// Last observed time.
    pub last_seen_at: Option<Timestamp>,
}

/// Account/session state returned by `GetAccountState`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct AccountState {
    /// Stable account identifier.
    pub account_id: Id,
    /// Account display name.
    pub display_name: Option<String>,
    /// Identities belonging to the account.
    pub identities: Vec<Identity>,
    /// Devices belonging to the account.
    pub devices: Vec<Device>,
    /// Current synchronization state.
    pub sync: SyncState,
}

/// A participant in a conversation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct Participant {
    /// Stable participant identifier.
    pub id: Id,
    /// Identity associated with this participant, if known.
    pub identity_id: Option<Id>,
    /// User-facing name.
    pub display_name: Option<String>,
    /// Participant role.
    pub role: ParticipantRole,
}

/// Conversation metadata and participants.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct Conversation {
    /// Stable conversation identifier.
    pub id: Id,
    /// Direct or group conversation.
    pub kind: ConversationKind,
    /// Optional title.
    pub title: Option<String>,
    /// Current participants.
    pub participants: Vec<Participant>,
    /// Most recent known update.
    pub updated_at: Option<Timestamp>,
    /// Number of unread messages known to the daemon.
    pub unread_count: u32,
}

/// A cursor-based conversation page.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct ConversationPage {
    /// Conversations in stable daemon order.
    pub conversations: Vec<Conversation>,
    /// Cursor for the next page, or `None` at the end.
    pub next_cursor: Option<String>,
}

/// A cursor-based request for conversations.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct ConversationPageRequest {
    /// Opaque cursor returned by the previous page.
    pub cursor: Option<String>,
    /// Maximum records requested.
    pub limit: u32,
}

impl ConversationPageRequest {
    /// Creates a validated page request.
    pub fn new(cursor: Option<String>, limit: u32) -> Result<Self, ProtocolError> {
        if limit == 0 || limit > MAX_PAGE_SIZE {
            return Err(ProtocolError::InvalidRequest(format!(
                "page size must be between 1 and {MAX_PAGE_SIZE}"
            )));
        }
        Ok(Self { cursor, limit })
    }
}

/// A text part sent or received in a message.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct TextPart {
    /// Text content.
    pub text: String,
    /// Optional UTF-16 formatting ranges, represented as `(start, end)` pairs.
    pub formatting: Vec<(u32, u32)>,
}

/// A reference to attachment metadata and daemon-controlled local content.
///
/// This type deliberately has no byte/body field. `location` is only emitted
/// after the daemon has authorized a local path or URI for the requesting
/// client, and `attachment_id` remains the stable key if the location expires.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct AttachmentReference {
    /// Stable attachment identifier.
    pub attachment_id: Id,
    /// Media category.
    pub kind: AttachmentKind,
    /// Optional original file name.
    pub file_name: Option<String>,
    /// Optional MIME type.
    pub mime_type: Option<String>,
    /// Optional byte size; the body is never included.
    pub byte_size: Option<u64>,
    /// Authorized local path or URI. `None` means content is not locally ready.
    pub location: Option<AttachmentLocation>,
}

impl AttachmentReference {
    /// Validates the stable ID and any local location.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.attachment_id.as_str().trim().is_empty() {
            return Err(ProtocolError::InvalidRequest(
                "attachment ID must not be empty".to_owned(),
            ));
        }
        if let Some(location) = &self.location {
            location.validate()?;
        }
        Ok(())
    }
}

/// An explicitly local attachment location. No remote URL is a valid value.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub enum AttachmentLocation {
    /// An absolute path controlled by the daemon.
    LocalPath(String),
    /// A local URI such as `file://` or a portal URI.
    LocalUri(String),
}

impl AttachmentLocation {
    /// Creates an authorized local path value.
    pub fn local_path(path: impl Into<String>) -> Result<Self, ProtocolError> {
        let path = path.into();
        let candidate = Path::new(&path);
        if !candidate.is_absolute()
            || candidate.components().any(|part| {
                matches!(part, std::path::Component::ParentDir)
                    || part == std::path::Component::CurDir
            })
            || path.contains('\0')
        {
            return Err(ProtocolError::InvalidRequest(
                "attachment path must be absolute and normalized".to_owned(),
            ));
        }
        Ok(Self::LocalPath(path))
    }

    /// Creates a URI value for an approved local URI scheme.
    pub fn local_uri(uri: impl Into<String>) -> Result<Self, ProtocolError> {
        let uri = uri.into();
        let allowed = ["file:///", "document-portal://", "content://"];
        if !allowed.iter().any(|prefix| uri.starts_with(prefix))
            || uri.chars().any(char::is_whitespace)
            || uri.contains('\0')
        {
            return Err(ProtocolError::InvalidRequest(
                "attachment URI must use an approved local scheme".to_owned(),
            ));
        }
        Ok(Self::LocalUri(uri))
    }

    /// Validates a location received over D-Bus.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        match self {
            Self::LocalPath(path) => Self::local_path(path.clone()).map(|_| ()),
            Self::LocalUri(uri) => Self::local_uri(uri.clone()).map(|_| ()),
        }
    }
}

/// A message part returned in history.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct MessagePart {
    /// The populated content field is selected by this kind.
    pub kind: MessagePartKind,
    /// Text and its formatting for a text part.
    pub text: Option<TextPart>,
    /// Metadata reference to an attachment body.
    pub attachment: Option<AttachmentReference>,
    /// A location represented without backend-specific types.
    pub location: Option<String>,
    /// A contact display label.
    pub contact: Option<String>,
    /// Link URL for a link part.
    pub link_url: Option<String>,
    /// Link title for a link part.
    pub link_title: Option<String>,
}

/// A message in a history page or change event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct Message {
    /// Stable message identifier.
    pub id: Id,
    /// Owning conversation.
    pub conversation_id: Id,
    /// Sender participant, if known.
    pub sender_id: Option<Id>,
    /// Message timestamp.
    pub sent_at: Timestamp,
    /// Ordered message parts.
    pub parts: Vec<MessagePart>,
    /// Current delivery state.
    pub delivery: DeliveryState,
    /// Current read state.
    pub read: ReadState,
    /// Reactions currently known for this message.
    pub reactions: Vec<Reaction>,
}

/// A cursor-based history request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct HistoryPageRequest {
    /// Conversation whose history is requested.
    pub conversation_id: Id,
    /// Opaque cursor returned by the previous page.
    pub cursor: Option<String>,
    /// Maximum records requested.
    pub limit: u32,
}

impl HistoryPageRequest {
    /// Creates a validated history request.
    pub fn new(
        conversation_id: Id,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<Self, ProtocolError> {
        ConversationPageRequest::new(cursor.clone(), limit)?;
        Ok(Self {
            conversation_id,
            cursor,
            limit,
        })
    }
}

/// A page of messages and its continuation cursor.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct HistoryPage {
    /// Messages in ascending daemon-defined history order.
    pub messages: Vec<Message>,
    /// Cursor for the next page, or `None` at the end.
    pub next_cursor: Option<String>,
}

/// Content accepted by `SendMessage`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct OutgoingPart {
    /// The populated content field is selected by this kind.
    pub kind: MessagePartKind,
    /// Text to send.
    pub text: Option<TextPart>,
    /// A previously registered/local attachment reference; never raw bytes.
    pub attachment: Option<AttachmentReference>,
}

/// Request to send one message.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct SendMessageRequest {
    /// Destination conversation.
    pub conversation_id: Id,
    /// Ordered text and attachment references.
    pub parts: Vec<OutgoingPart>,
    /// Client-generated idempotency key.
    pub client_mutation_id: Id,
}

/// Result of accepting a send request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct SendMessageResponse {
    /// Stable daemon/backend message identifier.
    pub message_id: Id,
    /// Current delivery state.
    pub delivery: DeliveryState,
    /// Time the daemon accepted the request.
    pub accepted_at: Timestamp,
}

/// Settings exposed through the protocol.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct Settings {
    /// Whether notifications are enabled and which ones are shown.
    pub notifications: NotificationMode,
    /// Whether message previews may appear in notifications.
    pub show_previews: bool,
    /// Whether attachment locations may be issued to this client.
    pub allow_local_attachment_locations: bool,
}

/// Partial settings update. `None` leaves a setting unchanged.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct UpdateSettingsRequest {
    /// New notification mode.
    pub notifications: Option<NotificationMode>,
    /// New message-preview preference.
    pub show_previews: Option<bool>,
    /// New local attachment-location preference.
    pub allow_local_attachment_locations: Option<bool>,
}

/// Current synchronization state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct SyncState {
    /// Whether an account is connected.
    pub connected: bool,
    /// Whether an initial sync has completed.
    pub initial_sync_complete: bool,
    /// Last successful synchronization time.
    pub last_success_at: Option<Timestamp>,
    /// Human-readable failure detail, if any.
    pub error: Option<String>,
}

/// Request for a state refresh. Refreshes are asynchronous.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct RefreshRequest {
    /// State scope to refresh.
    pub scope: RefreshScope,
    /// If set, refresh only this account.
    pub account_id: Option<Id>,
}

/// A refresh operation accepted by the daemon.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct RefreshResponse {
    /// Stable operation identifier used by sync events.
    pub operation_id: Id,
    /// Initial operation state.
    pub state: RefreshState,
}

/// A reaction attached to a message.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct Reaction {
    /// Stable reaction identifier.
    pub id: Id,
    /// Message receiving the reaction.
    pub message_id: Id,
    /// Participant who made the reaction.
    pub participant_id: Id,
    /// Reaction kind.
    pub kind: ReactionKind,
    /// Whether the reaction has been removed.
    pub removed: bool,
}

/// Common event metadata.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct EventMetadata {
    /// Monotonic daemon event identifier.
    pub event_id: Id,
    /// Event timestamp.
    pub occurred_at: Timestamp,
}

/// Account state change signal payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct AccountChanged {
    /// Event metadata.
    pub metadata: EventMetadata,
    /// New account state.
    pub account: AccountState,
}

/// Conversation state change signal payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct ConversationChanged {
    /// Event metadata.
    pub metadata: EventMetadata,
    /// New conversation state.
    pub conversation: Conversation,
}

/// Message add/change/remove signal payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct MessageChanged {
    /// Event metadata.
    pub metadata: EventMetadata,
    /// Conversation containing the message.
    pub conversation_id: Id,
    /// New message; `None` means it was removed.
    pub message: Option<Message>,
    /// Stable ID when a message was removed.
    pub removed_message_id: Option<Id>,
}

/// Reaction change signal payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct ReactionChanged {
    /// Event metadata.
    pub metadata: EventMetadata,
    /// New or removed reaction.
    pub reaction: Reaction,
}

/// Typing indicator signal payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct TypingChanged {
    /// Event metadata.
    pub metadata: EventMetadata,
    /// Conversation where typing was observed.
    pub conversation_id: Id,
    /// Participant whose typing state changed.
    pub participant_id: Id,
    /// New typing state.
    pub state: TypingState,
}

/// Read marker signal payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct ReadChanged {
    /// Event metadata.
    pub metadata: EventMetadata,
    /// Conversation containing the marker.
    pub conversation_id: Id,
    /// Message at the read boundary.
    pub message_id: Id,
    /// Participant whose read state changed.
    pub participant_id: Id,
    /// Read timestamp, if the marker was cleared.
    pub at: Option<Timestamp>,
}

/// Synchronization/refresh signal payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct SyncChanged {
    /// Event metadata.
    pub metadata: EventMetadata,
    /// Refresh operation identifier, if tied to a request.
    pub operation_id: Option<Id>,
    /// New sync state.
    pub state: SyncState,
}

/// Attachment availability signal payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct AttachmentChanged {
    /// Event metadata.
    pub metadata: EventMetadata,
    /// Updated attachment reference.
    pub attachment: AttachmentReference,
}

/// Call state signal payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct CallChanged {
    /// Event metadata.
    pub metadata: EventMetadata,
    /// Stable call identifier.
    pub call_id: Id,
    /// Conversation containing the call.
    pub conversation_id: Id,
    /// Call media kind.
    pub kind: CallKind,
    /// New call state.
    pub state: CallState,
    /// Participant IDs currently in the call.
    pub participant_ids: Vec<Id>,
}

/// Find My item change signal payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct FindMyChanged {
    /// Event metadata.
    pub metadata: EventMetadata,
    /// Stable Find My item identifier.
    pub item_id: Id,
    /// User-facing item name.
    pub name: String,
    /// New availability state.
    pub status: FindMyStatus,
    /// Optional location summary; exact location policy is daemon-controlled.
    pub location: Option<String>,
}

/// The single version-1 event signal vocabulary.
///
/// D-Bus structs require a stable signature, so the discriminator and all
/// event payload slots are carried together. Exactly one payload slot is
/// populated according to `kind`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
pub struct ProtocolEvent {
    /// Event variant.
    pub kind: EventKind,
    /// Account state change payload.
    pub account: Option<AccountChanged>,
    /// Conversation state change payload.
    pub conversation: Option<ConversationChanged>,
    /// Message state change payload.
    pub message: Option<MessageChanged>,
    /// Reaction state change payload.
    pub reaction: Option<ReactionChanged>,
    /// Typing state change payload.
    pub typing: Option<TypingChanged>,
    /// Read state change payload.
    pub read: Option<ReadChanged>,
    /// Synchronization state change payload.
    pub sync: Option<SyncChanged>,
    /// Attachment state change payload.
    pub attachment: Option<AttachmentChanged>,
    /// Call state change payload.
    pub call: Option<CallChanged>,
    /// Find My state change payload.
    pub find_my: Option<FindMyChanged>,
}

/// Typed client-side proxy for the version-1 D-Bus interface.
///
/// A daemon may implement the same method and signal signatures using
/// `#[zbus::interface(name = INTERFACE_NAME)]`; this proxy is provided so
/// later clients do not duplicate method names or wire types.
#[zbus::proxy(
    interface = "io.github.tannerkrewson.LiteBubbles.Backend.Protocol1",
    default_service = "io.github.tannerkrewson.LiteBubbles.Backend",
    default_path = "/io/github/tannerkrewson/LiteBubbles/Backend"
)]
pub trait LiteBubbles {
    /// Negotiate the protocol before making other calls.
    #[zbus(name = "Hello")]
    async fn hello(&self, request: HelloRequest) -> Result<HelloResponse, ProtocolError>;

    /// Return account/session state.
    #[zbus(name = "GetAccountState")]
    async fn get_account_state(&self) -> Result<AccountState, ProtocolError>;

    /// Return a page of conversations.
    #[zbus(name = "GetConversations")]
    async fn get_conversations(
        &self,
        request: ConversationPageRequest,
    ) -> Result<ConversationPage, ProtocolError>;

    /// Return a page of messages for one conversation.
    #[zbus(name = "GetHistory")]
    async fn get_history(&self, request: HistoryPageRequest) -> Result<HistoryPage, ProtocolError>;

    /// Accept a message containing text and/or attachment references.
    #[zbus(name = "SendMessage")]
    async fn send_message(
        &self,
        request: SendMessageRequest,
    ) -> Result<SendMessageResponse, ProtocolError>;

    /// Resolve metadata and an authorized local location for an attachment.
    #[zbus(name = "GetAttachment")]
    async fn get_attachment(&self, attachment_id: Id)
    -> Result<AttachmentReference, ProtocolError>;

    /// Return current client settings.
    #[zbus(name = "GetSettings")]
    async fn get_settings(&self) -> Result<Settings, ProtocolError>;

    /// Apply a partial settings update and return the resulting settings.
    #[zbus(name = "UpdateSettings")]
    async fn update_settings(
        &self,
        request: UpdateSettingsRequest,
    ) -> Result<Settings, ProtocolError>;

    /// Queue an asynchronous account/state refresh.
    #[zbus(name = "Refresh")]
    async fn refresh(&self, request: RefreshRequest) -> Result<RefreshResponse, ProtocolError>;

    /// Receive all version-1 asynchronous state changes.
    #[zbus(signal, name = "Event")]
    fn event(&self, event: ProtocolEvent) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use zvariant::{LE, serialized::Context, to_bytes};

    fn id(value: &str) -> Id {
        Id::new(value).expect("test ID is valid")
    }

    fn round_trip<T>(value: &T) -> T
    where
        T: for<'de> Deserialize<'de> + Serialize + Type + Eq + std::fmt::Debug,
    {
        let bytes = to_bytes(Context::new_dbus(LE, 0), value).expect("zvariant serializes");
        bytes.deserialize().expect("zvariant deserializes").0
    }

    #[test]
    fn protocol_types_round_trip_through_zvariant() {
        let request = HelloRequest {
            client_name: "test-client".to_owned(),
            min_version: 1,
            max_version: 3,
            capabilities: vec!["attachments".to_owned(), "events".to_owned()],
        };
        assert_eq!(round_trip(&request), request);

        let event = ProtocolEvent {
            kind: EventKind::AttachmentChanged,
            account: None,
            conversation: None,
            message: None,
            reaction: None,
            typing: None,
            read: None,
            sync: None,
            attachment: Some(AttachmentChanged {
                metadata: EventMetadata {
                    event_id: id("event-1"),
                    occurred_at: Timestamp(42),
                },
                attachment: AttachmentReference {
                    attachment_id: id("attachment-1"),
                    kind: AttachmentKind::Image,
                    file_name: Some("photo.jpg".to_owned()),
                    mime_type: Some("image/jpeg".to_owned()),
                    byte_size: Some(12),
                    location: Some(AttachmentLocation::LocalUri(
                        "file:///run/user/1000/litebubbles/photo.jpg".to_owned(),
                    )),
                },
            }),
            call: None,
            find_my: None,
        };
        assert_eq!(round_trip(&event), event);
    }

    #[test]
    fn negotiation_selects_highest_overlap_and_rejects_gaps() {
        let mut request = HelloRequest::new("client", 1, 4);
        assert!(matches!(negotiate_version(&request), Ok(1)));
        request.min_version = 2;
        assert!(matches!(
            negotiate_version(&request),
            Err(ProtocolError::UnsupportedVersion(_))
        ));
        request.min_version = 0;
        assert!(matches!(
            negotiate_version(&request),
            Err(ProtocolError::InvalidRequest(_))
        ));
    }

    #[test]
    fn typed_errors_have_stable_dbus_names() {
        use zbus::DBusError;

        let error = ProtocolError::NotFound("message-1".to_owned());
        assert_eq!(
            error.name().as_str(),
            "io.github.tannerkrewson.LiteBubbles.Backend.Protocol1.Error.NotFound"
        );
        assert_eq!(
            error.to_string(),
            "io.github.tannerkrewson.LiteBubbles.Backend.Protocol1.Error.NotFound: message-1"
        );
    }

    #[test]
    fn dbus_identifiers_match_application_identity() {
        assert_eq!(APPLICATION_ID, "io.github.tannerkrewson.LiteBubbles");
        assert_eq!(BUS_NAME, "io.github.tannerkrewson.LiteBubbles.Backend");
        assert_eq!(OBJECT_PATH, "/io/github/tannerkrewson/LiteBubbles/Backend");
        assert_eq!(
            INTERFACE_NAME,
            "io.github.tannerkrewson.LiteBubbles.Backend.Protocol1"
        );
    }

    #[test]
    fn attachment_locations_are_local_and_bodyless() {
        assert!(AttachmentLocation::local_path("relative/file").is_err());
        assert!(AttachmentLocation::local_path("/tmp/../private/file").is_err());
        assert!(AttachmentLocation::local_uri("https://example.invalid/file").is_err());
        assert!(AttachmentLocation::local_uri("file:///tmp/file").is_ok());

        let reference = AttachmentReference {
            attachment_id: id("attachment-1"),
            kind: AttachmentKind::File,
            file_name: None,
            mime_type: None,
            byte_size: Some(100),
            location: Some(AttachmentLocation::LocalPath("/tmp/file".to_owned())),
        };
        assert!(reference.validate().is_ok());
    }

    #[test]
    fn page_requests_enforce_the_protocol_limit() {
        assert!(ConversationPageRequest::new(None, 0).is_err());
        assert!(ConversationPageRequest::new(None, MAX_PAGE_SIZE + 1).is_err());
        assert!(ConversationPageRequest::new(None, MAX_PAGE_SIZE).is_ok());
    }
}
