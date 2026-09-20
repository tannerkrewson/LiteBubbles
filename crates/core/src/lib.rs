//! Backend-independent Apple-semantic domain types.
//!
//! The core crate owns the meaning and relationships of synchronized data. It
//! deliberately does not contain protocol, storage, UI, or backend types.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            /// Creates an identifier. Backend identifiers are opaque, but may
            /// not be empty or consist only of whitespace.
            pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
                let value = value.into();
                if value.trim().is_empty() {
                    return Err(IdError::Empty);
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = IdError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl FromStr for $name {
            type Err = IdError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::try_from(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                struct IdVisitor;

                impl<'de> de::Visitor<'de> for IdVisitor {
                    type Value = $name;

                    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                        formatter.write_str("a non-empty opaque identifier")
                    }

                    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
                    where
                        E: de::Error,
                    {
                        $name::new(value).map_err(E::custom)
                    }

                    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
                    where
                        E: de::Error,
                    {
                        $name::new(value).map_err(E::custom)
                    }
                }

                deserializer.deserialize_string(IdVisitor)
            }
        }
    };
}

string_id!(AccountId);
string_id!(IdentityId);
string_id!(PersonId);
string_id!(ConversationId);
string_id!(ParticipantId);
string_id!(MessageId);
string_id!(MessagePartId);
string_id!(AttachmentId);
string_id!(MessageMutationId);
string_id!(ReactionId);
string_id!(CallId);
string_id!(DeviceId);
string_id!(FindMyItemId);
string_id!(SharedAlbumId);
string_id!(SharedAlbumAssetId);
string_id!(BackendEventId);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IdError {
    Empty,
}

impl fmt::Display for IdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("identifier must not be empty"),
        }
    }
}

impl std::error::Error for IdError {}

/// Milliseconds since the Unix epoch. The core does not depend on a date/time
/// implementation and can therefore be used by any backend.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
pub struct Timestamp(pub i64);

impl Timestamp {
    pub const fn from_millis(value: i64) -> Self {
        Self(value)
    }

    pub const fn as_millis(self) -> i64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DomainError {
    InvalidRange,
    DuplicateId(&'static str),
    MissingRelationship(&'static str),
    InvalidRelationship(&'static str),
}

impl fmt::Display for DomainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRange => formatter.write_str("text formatting range is invalid"),
            Self::DuplicateId(kind) => write!(formatter, "duplicate {kind} identifier"),
            Self::MissingRelationship(kind) => write!(formatter, "missing {kind} relationship"),
            Self::InvalidRelationship(kind) => write!(formatter, "invalid {kind} relationship"),
        }
    }
}

impl std::error::Error for DomainError {}

/// Opaque data supplied by a service-specific extension. The core preserves it
/// without interpreting service or storage details.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceExtension {
    pub service: String,
    pub name: String,
    pub content_type: Option<String>,
    pub payload: Vec<u8>,
}

impl ServiceExtension {
    pub fn new(
        service: impl Into<String>,
        name: impl Into<String>,
        content_type: Option<String>,
        payload: Vec<u8>,
    ) -> Result<Self, DomainError> {
        let service = service.into();
        let name = name.into();
        if service.trim().is_empty() || name.trim().is_empty() {
            return Err(DomainError::InvalidRelationship("service extension name"));
        }
        Ok(Self {
            service,
            name,
            content_type,
            payload,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum IdentityAddress {
    Email(String),
    Phone(String),
    AppleAccount(String),
    Other { kind: String, value: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Identity {
    pub id: IdentityId,
    pub address: IdentityAddress,
    pub person_id: Option<PersonId>,
    pub label: Option<String>,
    pub extensions: Vec<ServiceExtension>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Person {
    pub id: PersonId,
    pub display_name: String,
    pub identity_ids: Vec<IdentityId>,
    pub avatar: Option<Attachment>,
    pub extensions: Vec<ServiceExtension>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Account {
    pub id: AccountId,
    pub display_name: Option<String>,
    pub identities: Vec<Identity>,
    pub devices: Vec<Device>,
    pub extensions: Vec<ServiceExtension>,
}

impl Account {
    pub fn validate(&self) -> Result<(), DomainError> {
        ensure_unique(
            self.identities.iter().map(|identity| &identity.id),
            "identity",
        )?;
        ensure_unique(self.devices.iter().map(|device| &device.id), "device")?;
        for device in &self.devices {
            if device.account_id != self.id {
                return Err(DomainError::InvalidRelationship("device account"));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Participant {
    pub id: ParticipantId,
    pub person_id: Option<PersonId>,
    pub identity_id: Option<IdentityId>,
    pub display_name: Option<String>,
    pub role: ParticipantRole,
    pub joined_at: Option<Timestamp>,
    pub left_at: Option<Timestamp>,
    pub extensions: Vec<ServiceExtension>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ParticipantRole {
    Member,
    Owner,
    Administrator,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ConversationKind {
    Direct,
    Group(Box<GroupDetails>),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GroupDetails {
    pub title: Option<String>,
    pub owner: Option<ParticipantId>,
    pub avatar: Option<Attachment>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Conversation {
    pub id: ConversationId,
    pub kind: ConversationKind,
    pub title: Option<String>,
    pub participants: Vec<Participant>,
    pub created_at: Option<Timestamp>,
    pub updated_at: Option<Timestamp>,
    pub extensions: Vec<ServiceExtension>,
}

impl Conversation {
    pub fn validate(&self) -> Result<(), DomainError> {
        ensure_unique(
            self.participants.iter().map(|participant| &participant.id),
            "participant",
        )?;
        if let ConversationKind::Group(details) = &self.kind {
            if let Some(owner) = &details.owner {
                if !self
                    .participants
                    .iter()
                    .any(|participant| &participant.id == owner)
                {
                    return Err(DomainError::InvalidRelationship("group owner"));
                }
            }
        }
        Ok(())
    }

    pub fn participant(&self, id: &ParticipantId) -> Option<&Participant> {
        self.participants
            .iter()
            .find(|participant| &participant.id == id)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TextPart {
    pub text: String,
    /// Ranges use UTF-16 code-unit offsets, matching Apple's message
    /// formatting representation. Spans may overlap.
    pub formatting: Vec<TextFormatting>,
}

impl TextPart {
    pub fn validate(&self) -> Result<(), DomainError> {
        let text_len = self.text.encode_utf16().count() as u32;
        for formatting in &self.formatting {
            if formatting.range.end > text_len || formatting.range.start > formatting.range.end {
                return Err(DomainError::InvalidRange);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TextFormatting {
    pub range: TextRange,
    pub style: TextStyle,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TextRange {
    pub start: u32,
    pub end: u32,
}

impl TextRange {
    pub const fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum TextStyle {
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Monospace,
    Link { url: String },
    Mention { participant_id: ParticipantId },
    Color { value: String },
    Custom(ServiceExtension),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum AttachmentKind {
    Image,
    Video,
    Audio,
    File,
    Sticker,
    Contact,
    Location,
    Other(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum AttachmentContent {
    Inline(Vec<u8>),
    /// An opaque content token. Its interpretation belongs to the backend or
    /// application and is intentionally not a path or storage handle.
    External {
        token: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Attachment {
    pub id: AttachmentId,
    pub kind: AttachmentKind,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub byte_size: Option<u64>,
    pub content: AttachmentContent,
    pub extensions: Vec<ServiceExtension>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AttachmentPart {
    pub attachment: Attachment,
    pub caption: Option<TextPart>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LinkPreview {
    pub url: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub image: Option<Attachment>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContactCard {
    pub person: Option<PersonId>,
    pub display_name: String,
    pub identities: Vec<IdentityAddress>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum MessagePart {
    Text(TextPart),
    Attachment(AttachmentPart),
    LinkPreview(LinkPreview),
    Location(Location),
    Contact(ContactCard),
    ServiceExtension(ServiceExtension),
}

impl MessagePart {
    pub fn validate(&self) -> Result<(), DomainError> {
        match self {
            Self::Text(text) => text.validate(),
            Self::Attachment(part) => part.caption.as_ref().map_or(Ok(()), TextPart::validate),
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum MessageMutationKind {
    Edit { parts: Vec<MessagePart> },
    Unsend { reason: Option<String> },
    ServiceExtension(ServiceExtension),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MessageMutation {
    pub id: MessageMutationId,
    pub message_id: MessageId,
    pub actor: Option<ParticipantId>,
    pub occurred_at: Timestamp,
    pub kind: MessageMutationKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum DeliveryState {
    Queued,
    Sent,
    Delivered,
    Failed { reason: Option<String> },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeliveryReceipt {
    pub participant_id: ParticipantId,
    pub state: DeliveryState,
    pub at: Option<Timestamp>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ReadState {
    Unread,
    Read { at: Timestamp },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReadReceipt {
    pub participant_id: ParticipantId,
    pub at: Timestamp,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ReactionKind {
    Love,
    Like,
    Dislike,
    Laugh,
    Emphasize,
    Question,
    Custom(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Reaction {
    pub id: ReactionId,
    pub message_id: MessageId,
    pub participant_id: ParticipantId,
    pub kind: ReactionKind,
    pub created_at: Timestamp,
    pub removed_at: Option<Timestamp>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Message {
    pub id: MessageId,
    pub conversation_id: ConversationId,
    pub sender: Option<ParticipantId>,
    pub sent_at: Timestamp,
    /// Parts remain ordered because text, media, links, and extensions can be
    /// interleaved in a single Apple message.
    pub parts: Vec<MessagePart>,
    pub mutations: Vec<MessageMutation>,
    pub reactions: Vec<Reaction>,
    pub delivery: DeliveryState,
    pub delivery_receipts: Vec<DeliveryReceipt>,
    pub read_state: ReadState,
    pub read_receipts: Vec<ReadReceipt>,
    pub reply_to: Option<MessageId>,
    pub extensions: Vec<ServiceExtension>,
}

impl Message {
    pub fn validate(&self) -> Result<(), DomainError> {
        ensure_unique(
            self.mutations.iter().map(|mutation| &mutation.id),
            "message mutation",
        )?;
        ensure_unique(
            self.reactions.iter().map(|reaction| &reaction.id),
            "reaction",
        )?;
        ensure_unique(
            self.delivery_receipts
                .iter()
                .map(|receipt| &receipt.participant_id),
            "delivery receipt",
        )?;
        ensure_unique(
            self.read_receipts
                .iter()
                .map(|receipt| &receipt.participant_id),
            "read receipt",
        )?;
        for part in &self.parts {
            part.validate()?;
        }
        for mutation in &self.mutations {
            if mutation.message_id != self.id {
                return Err(DomainError::InvalidRelationship("message mutation"));
            }
            if let MessageMutationKind::Edit { parts } = &mutation.kind {
                for part in parts {
                    part.validate()?;
                }
            }
        }
        for reaction in &self.reactions {
            if reaction.message_id != self.id {
                return Err(DomainError::InvalidRelationship("reaction message"));
            }
        }
        Ok(())
    }

    pub fn is_unsent(&self) -> bool {
        self.mutations
            .iter()
            .any(|mutation| matches!(mutation.kind, MessageMutationKind::Unsend { .. }))
    }
}

impl Ord for Message {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.sent_at
            .cmp(&other.sent_at)
            .then_with(|| self.id.cmp(&other.id))
    }
}

impl PartialOrd for Message {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum TypingState {
    Started,
    Stopped,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TypingIndicator {
    pub conversation_id: ConversationId,
    pub participant_id: ParticipantId,
    pub state: TypingState,
    pub observed_at: Timestamp,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum CallKind {
    Audio,
    Video,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum CallState {
    Ringing,
    Active,
    Held,
    Ended,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum CallParticipantState {
    Invited,
    Connecting,
    Connected,
    Declined,
    Disconnected,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CallParticipant {
    pub participant_id: ParticipantId,
    pub state: CallParticipantState,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Call {
    pub id: CallId,
    pub conversation_id: ConversationId,
    pub kind: CallKind,
    pub state: CallState,
    pub participants: Vec<CallParticipant>,
    pub started_at: Option<Timestamp>,
    pub ended_at: Option<Timestamp>,
    pub extensions: Vec<ServiceExtension>,
}

impl Call {
    pub fn validate(&self) -> Result<(), DomainError> {
        ensure_unique(
            self.participants
                .iter()
                .map(|participant| &participant.participant_id),
            "call participant",
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum DeviceKind {
    Phone,
    Tablet,
    Computer,
    Watch,
    Other(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum DeviceCapability {
    Messaging,
    AudioCalls,
    VideoCalls,
    FindMy,
    SharedAlbums,
    Other(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Device {
    pub id: DeviceId,
    pub account_id: AccountId,
    pub identity_id: Option<IdentityId>,
    pub name: Option<String>,
    pub kind: DeviceKind,
    pub capabilities: Vec<DeviceCapability>,
    pub last_seen_at: Option<Timestamp>,
    pub extensions: Vec<ServiceExtension>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GeoPoint {
    /// Latitude and longitude in millionths of a degree.
    pub latitude_e6: i32,
    pub longitude_e6: i32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Location {
    pub point: GeoPoint,
    pub accuracy_meters: Option<u32>,
    pub observed_at: Option<Timestamp>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum FindMyItemKind {
    Device,
    AirTag,
    Vehicle,
    Accessory,
    Other(String),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum FindMyItemStatus {
    Online,
    Offline,
    Lost,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FindMyItem {
    pub id: FindMyItemId,
    pub owner: AccountId,
    pub name: String,
    pub kind: FindMyItemKind,
    pub status: FindMyItemStatus,
    pub location: Option<Location>,
    pub battery_percent: Option<u8>,
    pub updated_at: Option<Timestamp>,
    pub extensions: Vec<ServiceExtension>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum AlbumAssetKind {
    Photo,
    Video,
    Other,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SharedAlbumMember {
    pub person_id: PersonId,
    pub can_contribute: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SharedAlbumAsset {
    pub id: SharedAlbumAssetId,
    pub album_id: SharedAlbumId,
    pub kind: AlbumAssetKind,
    pub attachment: Attachment,
    pub caption: Option<TextPart>,
    pub contributed_by: Option<PersonId>,
    pub created_at: Option<Timestamp>,
    pub extensions: Vec<ServiceExtension>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SharedAlbum {
    pub id: SharedAlbumId,
    pub title: String,
    pub owner: PersonId,
    pub members: Vec<SharedAlbumMember>,
    pub assets: Vec<SharedAlbumAsset>,
    pub created_at: Option<Timestamp>,
    pub extensions: Vec<ServiceExtension>,
}

impl SharedAlbum {
    pub fn validate(&self) -> Result<(), DomainError> {
        ensure_unique(
            self.members.iter().map(|member| &member.person_id),
            "shared album member",
        )?;
        ensure_unique(
            self.assets.iter().map(|asset| &asset.id),
            "shared album asset",
        )?;
        for asset in &self.assets {
            if asset.album_id != self.id {
                return Err(DomainError::InvalidRelationship("shared album asset"));
            }
            if let Some(caption) = &asset.caption {
                caption.validate()?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum BackendEventKind {
    AccountChanged(Account),
    PersonChanged(Person),
    ConversationChanged(Conversation),
    MessageAdded(Message),
    MessageChanged(Message),
    MessageRemoved {
        conversation_id: ConversationId,
        message_id: MessageId,
    },
    TypingChanged(TypingIndicator),
    CallChanged(Call),
    DeviceChanged(Device),
    FindMyItemChanged(FindMyItem),
    SharedAlbumChanged(SharedAlbum),
    ServiceExtension(ServiceExtension),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BackendEvent {
    pub id: BackendEventId,
    pub occurred_at: Timestamp,
    pub kind: BackendEventKind,
    pub extensions: Vec<ServiceExtension>,
}

fn ensure_unique<'a, I, T>(values: I, kind: &'static str) -> Result<(), DomainError>
where
    I: IntoIterator<Item = &'a T>,
    T: Eq + std::hash::Hash + 'a,
{
    let mut seen: std::collections::HashSet<&'a T> = std::collections::HashSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(DomainError::DuplicateId(kind));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id<T>(value: &str) -> T
    where
        T: TryFrom<String, Error = IdError>,
    {
        match T::try_from(value.to_owned()) {
            Ok(value) => value,
            Err(error) => panic!("test id should be valid: {error}"),
        }
    }

    fn text(value: &str) -> MessagePart {
        MessagePart::Text(TextPart {
            text: value.to_owned(),
            formatting: Vec::new(),
        })
    }

    #[test]
    fn ids_are_typed_non_empty_and_serde_validated() {
        let conversation: ConversationId = id("conversation");
        let message: MessageId = id("conversation");
        assert_eq!(conversation.as_str(), message.as_str());
        assert_ne!(conversation, id::<ConversationId>("other"));
        assert!(ConversationId::new("   ").is_err());

        let encoded = match serde_json::to_string(&conversation) {
            Ok(encoded) => encoded,
            Err(error) => panic!("id serializes: {error}"),
        };
        assert_eq!(encoded, "\"conversation\"");
        let decoded: Result<ConversationId, _> = serde_json::from_str("\"\"");
        assert!(decoded.is_err());
    }

    #[test]
    fn text_formatting_uses_utf16_bounds() {
        let valid = TextPart {
            text: "a😀b".to_owned(),
            formatting: vec![TextFormatting {
                range: TextRange::new(1, 3),
                style: TextStyle::Bold,
            }],
        };
        assert!(valid.validate().is_ok());
        let invalid = TextPart {
            formatting: vec![TextFormatting {
                range: TextRange::new(0, 5),
                style: TextStyle::Italic,
            }],
            ..valid
        };
        assert_eq!(invalid.validate(), Err(DomainError::InvalidRange));
    }

    #[test]
    fn multipart_message_preserves_order_and_representative_parts() {
        let attachment = Attachment {
            id: id("attachment"),
            kind: AttachmentKind::Image,
            file_name: Some("photo.jpg".to_owned()),
            mime_type: Some("image/jpeg".to_owned()),
            byte_size: Some(3),
            content: AttachmentContent::Inline(vec![1, 2, 3]),
            extensions: Vec::new(),
        };
        let message = Message {
            id: id("message"),
            conversation_id: id("conversation"),
            sender: Some(id("participant")),
            sent_at: Timestamp::from_millis(1),
            parts: vec![
                text("before"),
                MessagePart::Attachment(AttachmentPart {
                    attachment,
                    caption: Some(TextPart {
                        text: "caption".to_owned(),
                        formatting: Vec::new(),
                    }),
                }),
                MessagePart::LinkPreview(LinkPreview {
                    url: "https://example.test".to_owned(),
                    title: Some("Example".to_owned()),
                    summary: None,
                    image: None,
                }),
            ],
            mutations: Vec::new(),
            reactions: Vec::new(),
            delivery: DeliveryState::Sent,
            delivery_receipts: Vec::new(),
            read_state: ReadState::Unread,
            read_receipts: Vec::new(),
            reply_to: None,
            extensions: Vec::new(),
        };
        assert!(message.validate().is_ok());
        assert!(matches!(message.parts[0], MessagePart::Text(_)));
        assert!(matches!(message.parts[1], MessagePart::Attachment(_)));
        assert!(matches!(message.parts[2], MessagePart::LinkPreview(_)));
    }

    #[test]
    fn message_mutations_and_reactions_must_point_to_their_owner() {
        let message_id = id::<MessageId>("message");
        let mut message = Message {
            id: message_id.clone(),
            conversation_id: id("conversation"),
            sender: None,
            sent_at: Timestamp::from_millis(10),
            parts: vec![text("current")],
            mutations: vec![MessageMutation {
                id: id("edit"),
                message_id: message_id.clone(),
                actor: None,
                occurred_at: Timestamp::from_millis(11),
                kind: MessageMutationKind::Edit {
                    parts: vec![text("edited")],
                },
            }],
            reactions: vec![Reaction {
                id: id("reaction"),
                message_id: message_id.clone(),
                participant_id: id("participant"),
                kind: ReactionKind::Like,
                created_at: Timestamp::from_millis(12),
                removed_at: None,
            }],
            delivery: DeliveryState::Delivered,
            delivery_receipts: Vec::new(),
            read_state: ReadState::Unread,
            read_receipts: Vec::new(),
            reply_to: None,
            extensions: Vec::new(),
        };
        assert!(message.validate().is_ok());
        message.mutations[0].message_id = id("other-message");
        assert_eq!(
            message.validate(),
            Err(DomainError::InvalidRelationship("message mutation"))
        );
    }

    #[test]
    fn conversations_validate_group_ownership_and_participant_uniqueness() {
        let owner = Participant {
            id: id("owner"),
            person_id: Some(id("person")),
            identity_id: None,
            display_name: Some("Owner".to_owned()),
            role: ParticipantRole::Owner,
            joined_at: None,
            left_at: None,
            extensions: Vec::new(),
        };
        let mut conversation = Conversation {
            id: id("conversation"),
            kind: ConversationKind::Group(Box::new(GroupDetails {
                title: Some("Friends".to_owned()),
                owner: Some(owner.id.clone()),
                avatar: None,
            })),
            title: Some("Friends".to_owned()),
            participants: vec![owner.clone()],
            created_at: None,
            updated_at: None,
            extensions: Vec::new(),
        };
        assert!(conversation.validate().is_ok());
        conversation.participants.push(owner);
        assert_eq!(
            conversation.validate(),
            Err(DomainError::DuplicateId("participant"))
        );
    }

    #[test]
    fn messages_order_by_timestamp_then_stable_id() {
        let make_message = |id_value: &str, timestamp| Message {
            id: id(id_value),
            conversation_id: id("conversation"),
            sender: None,
            sent_at: Timestamp::from_millis(timestamp),
            parts: Vec::new(),
            mutations: Vec::new(),
            reactions: Vec::new(),
            delivery: DeliveryState::Queued,
            delivery_receipts: Vec::new(),
            read_state: ReadState::Unread,
            read_receipts: Vec::new(),
            reply_to: None,
            extensions: Vec::new(),
        };
        let later = make_message("a", 2);
        let earlier = make_message("z", 1);
        let same_time_lower_id = make_message("a", 2);
        assert!(earlier < later);
        assert_eq!(later, same_time_lower_id);
    }

    #[test]
    fn shared_album_assets_keep_album_relationship() {
        let album_id = id::<SharedAlbumId>("album");
        let attachment = Attachment {
            id: id("photo"),
            kind: AttachmentKind::Image,
            file_name: None,
            mime_type: None,
            byte_size: None,
            content: AttachmentContent::External {
                token: "opaque-photo".to_owned(),
            },
            extensions: Vec::new(),
        };
        let mut album = SharedAlbum {
            id: album_id.clone(),
            title: "Trip".to_owned(),
            owner: id("person"),
            members: Vec::new(),
            assets: vec![SharedAlbumAsset {
                id: id("asset"),
                album_id,
                kind: AlbumAssetKind::Photo,
                attachment,
                caption: None,
                contributed_by: None,
                created_at: None,
                extensions: Vec::new(),
            }],
            created_at: None,
            extensions: Vec::new(),
        };
        assert!(album.validate().is_ok());
        album.assets[0].album_id = id("other-album");
        assert_eq!(
            album.validate(),
            Err(DomainError::InvalidRelationship("shared album asset"))
        );
    }
}
