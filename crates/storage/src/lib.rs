//! SQLite persistence for the backend-independent LiteBubbles domain model.
//!
//! Only `litebubbles-core` values cross this boundary. GTK objects and
//! service/backend structs are intentionally not serialised here.

use std::{path::Path, str::FromStr};

use litebubbles_core::{
    Account, AccountId, Attachment, AttachmentContent, Conversation, ConversationId,
    ConversationKind, DeliveryReceipt, Device, DeviceId, DomainError, Identity, IdentityAddress,
    IdentityId, Message, MessageId, MessageMutation, MessagePart, Participant, ParticipantId,
    PersonId, Reaction, ReadReceipt, ReadState, ServiceExtension, Timestamp,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

const MIGRATIONS: &[(u32, &str)] = &[
    (1, include_str!("../migrations/0001_initial.sql")),
    (2, include_str!("../migrations/0002_indexes.sql")),
];

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("failed to open SQLite database at {path}: {source}")]
    Open {
        path: String,
        #[source]
        source: rusqlite::Error,
    },
    #[error("SQLite operation failed in {context}: {source}")]
    Sql {
        context: &'static str,
        #[source]
        source: rusqlite::Error,
    },
    #[error("migration {version} failed: {source}")]
    Migration {
        version: u32,
        #[source]
        source: rusqlite::Error,
    },
    #[error("database schema version {found} is newer than this storage library")]
    NewerSchema { found: u32 },
    #[error("failed to encode {entity}: {source}")]
    Encode {
        entity: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to decode stored {entity}: {source}")]
    Decode {
        entity: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("stored {entity} is invalid: {message}")]
    Corrupt {
        entity: &'static str,
        message: String,
    },
    #[error("invalid {entity}: {source}")]
    Domain {
        entity: &'static str,
        #[source]
        source: DomainError,
    },
    #[error("page limit must be greater than zero")]
    InvalidPageLimit,
}

fn sql(context: &'static str, source: rusqlite::Error) -> StorageError {
    StorageError::Sql { context, source }
}
fn corrupt(entity: &'static str, message: impl Into<String>) -> StorageError {
    StorageError::Corrupt {
        entity,
        message: message.into(),
    }
}
fn encode<T: Serialize>(entity: &'static str, value: &T) -> Result<String, StorageError> {
    serde_json::to_string(value).map_err(|source| StorageError::Encode { entity, source })
}
fn decode<T: DeserializeOwned>(entity: &'static str, value: &str) -> Result<T, StorageError> {
    serde_json::from_str(value).map_err(|source| StorageError::Decode { entity, source })
}
fn domain(entity: &'static str, result: Result<(), DomainError>) -> Result<(), StorageError> {
    result.map_err(|source| StorageError::Domain { entity, source })
}
fn millis(value: Option<Timestamp>) -> Option<i64> {
    value.map(Timestamp::as_millis)
}
fn timestamp(value: Option<i64>) -> Option<Timestamp> {
    value.map(Timestamp::from_millis)
}
fn account_key(id: Option<&AccountId>) -> &str {
    id.map_or("", AccountId::as_str)
}
fn id<T: FromStr>(value: String, entity: &'static str) -> Result<T, StorageError>
where
    T::Err: std::fmt::Display,
{
    value
        .parse::<T>()
        .map_err(|error| corrupt(entity, error.to_string()))
}

fn u64_i64(entity: &'static str, value: Option<u64>) -> Result<Option<i64>, StorageError> {
    value
        .map(|value| {
            i64::try_from(value).map_err(|_| corrupt(entity, "value exceeds SQLite integer range"))
        })
        .transpose()
}
fn i64_u64(entity: &'static str, value: Option<i64>) -> Result<Option<u64>, StorageError> {
    value
        .map(|value| u64::try_from(value).map_err(|_| corrupt(entity, "negative value")))
        .transpose()
}

fn identity_parts(address: &IdentityAddress) -> (&'static str, &str, Option<&str>) {
    match address {
        IdentityAddress::Email(value) => ("email", value, None),
        IdentityAddress::Phone(value) => ("phone", value, None),
        IdentityAddress::AppleAccount(value) => ("apple_account", value, None),
        IdentityAddress::Other { kind, value } => ("other", value, Some(kind)),
    }
}
fn identity_address(
    kind: String,
    value: String,
    extra: Option<String>,
) -> Result<IdentityAddress, StorageError> {
    match kind.as_str() {
        "email" => Ok(IdentityAddress::Email(value)),
        "phone" => Ok(IdentityAddress::Phone(value)),
        "apple_account" => Ok(IdentityAddress::AppleAccount(value)),
        "other" => Ok(IdentityAddress::Other {
            kind: extra.ok_or_else(|| corrupt("identity address", "missing kind"))?,
            value,
        }),
        _ => Err(corrupt("identity address", format!("unknown kind {kind}"))),
    }
}

fn save_extensions(
    tx: &Transaction<'_>,
    kind: &str,
    id: &str,
    values: &[ServiceExtension],
) -> Result<(), StorageError> {
    tx.execute(
        "DELETE FROM extensions WHERE owner_kind = ?1 AND owner_id = ?2",
        params![kind, id],
    )
    .map_err(|source| sql("replace extensions", source))?;
    for (ordinal, value) in values.iter().enumerate() {
        tx.execute(
            "INSERT INTO extensions(owner_kind, owner_id, ordinal, service, name, content_type, payload)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![kind, id, i64::try_from(ordinal).map_err(|_| corrupt("extension", "too many values"))?, value.service, value.name, value.content_type, value.payload],
        ).map_err(|source| sql("insert extension", source))?;
    }
    Ok(())
}
fn load_extensions(
    connection: &Connection,
    kind: &str,
    id: &str,
) -> Result<Vec<ServiceExtension>, StorageError> {
    let mut statement = connection.prepare("SELECT service, name, content_type, payload FROM extensions WHERE owner_kind = ?1 AND owner_id = ?2 ORDER BY ordinal")
        .map_err(|source| sql("prepare extension query", source))?;
    statement
        .query_map(params![kind, id], |row| {
            Ok(ServiceExtension {
                service: row.get(0)?,
                name: row.get(1)?,
                content_type: row.get(2)?,
                payload: row.get(3)?,
            })
        })
        .map_err(|source| sql("query extensions", source))?
        .map(|row| row.map_err(|source| sql("read extension", source)))
        .collect()
}

fn save_attachment(tx: &Transaction<'_>, attachment: &Attachment) -> Result<(), StorageError> {
    let (content_kind, content) = match &attachment.content {
        AttachmentContent::Inline(bytes) => ("inline", bytes.clone()),
        AttachmentContent::External { token } => ("external", token.as_bytes().to_vec()),
    };
    tx.execute(
        "INSERT INTO attachments(id, kind_json, file_name, mime_type, byte_size, content_kind, content)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET kind_json=excluded.kind_json, file_name=excluded.file_name,
         mime_type=excluded.mime_type, byte_size=excluded.byte_size, content_kind=excluded.content_kind, content=excluded.content",
        params![attachment.id.as_str(), encode("attachment kind", &attachment.kind)?, attachment.file_name, attachment.mime_type,
            u64_i64("attachment", attachment.byte_size)?, content_kind, content],
    ).map_err(|source| sql("upsert attachment", source))?;
    save_extensions(
        tx,
        "attachment",
        attachment.id.as_str(),
        &attachment.extensions,
    )
}
fn load_attachment(
    connection: &Connection,
    id_value: &str,
) -> Result<Option<Attachment>, StorageError> {
    let row = connection.query_row(
        "SELECT kind_json, file_name, mime_type, byte_size, content_kind, content FROM attachments WHERE id = ?1",
        params![id_value],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, Option<i64>>(3)?, row.get::<_, String>(4)?, row.get::<_, Vec<u8>>(5)?)),
    ).optional().map_err(|source| sql("load attachment", source))?;
    let Some((kind, file_name, mime_type, size, content_kind, content)) = row else {
        return Ok(None);
    };
    let content = match content_kind.as_str() {
        "inline" => AttachmentContent::Inline(content),
        "external" => AttachmentContent::External {
            token: String::from_utf8(content)
                .map_err(|error| corrupt("attachment content", error.to_string()))?,
        },
        _ => return Err(corrupt("attachment content", "unknown content kind")),
    };
    Ok(Some(Attachment {
        id: id(id_value.to_owned(), "attachment")?,
        kind: decode("attachment kind", &kind)?,
        file_name,
        mime_type,
        byte_size: i64_u64("attachment", size)?,
        content,
        extensions: load_extensions(connection, "attachment", id_value)?,
    }))
}
fn save_part_attachments(tx: &Transaction<'_>, part: &MessagePart) -> Result<(), StorageError> {
    match part {
        MessagePart::Attachment(value) => save_attachment(tx, &value.attachment),
        MessagePart::LinkPreview(value) => value
            .image
            .as_ref()
            .map_or(Ok(()), |image| save_attachment(tx, image)),
        _ => Ok(()),
    }
}
fn part_kind(part: &MessagePart) -> &'static str {
    match part {
        MessagePart::Text(_) => "text",
        MessagePart::Attachment(_) => "attachment",
        MessagePart::LinkPreview(_) => "link_preview",
        MessagePart::Location(_) => "location",
        MessagePart::Contact(_) => "contact",
        MessagePart::ServiceExtension(_) => "service_extension",
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageCursor {
    pub sent_at: Timestamp,
    pub id: MessageId,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessagePage {
    pub limit: u32,
    pub after: Option<MessageCursor>,
    pub before: Option<MessageCursor>,
}
impl MessagePage {
    pub fn new(limit: u32) -> Result<Self, StorageError> {
        if limit == 0 {
            return Err(StorageError::InvalidPageLimit);
        }
        Ok(Self {
            limit,
            after: None,
            before: None,
        })
    }
    pub fn after(mut self, cursor: MessageCursor) -> Self {
        self.after = Some(cursor);
        self.before = None;
        self
    }
    pub fn before(mut self, cursor: MessageCursor) -> Self {
        self.before = Some(cursor);
        self.after = None;
        self
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyncMetadata {
    pub service: String,
    pub account_id: Option<AccountId>,
    pub cursor: Option<String>,
    pub last_synced_at: Option<Timestamp>,
    pub updated_at: Timestamp,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceSyncState {
    pub service: String,
    pub account_id: Option<AccountId>,
    pub key: String,
    pub value: Vec<u8>,
    pub updated_at: Timestamp,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnreadState {
    pub conversation_id: ConversationId,
    pub participant_id: Option<ParticipantId>,
    pub unread_count: u64,
    pub last_read_at: Option<Timestamp>,
}

pub struct Store {
    connection: Connection,
}

impl Store {
    pub fn open() -> Result<Self, StorageError> {
        Self::open_path("litebubbles.sqlite3")
    }
    pub fn open_path(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref();
        let connection = Connection::open(path).map_err(|source| StorageError::Open {
            path: path.display().to_string(),
            source,
        })?;
        Self::from_connection(connection)
    }
    pub fn open_in_memory() -> Result<Self, StorageError> {
        Self::from_connection(Connection::open_in_memory().map_err(|source| {
            StorageError::Open {
                path: ":memory:".to_owned(),
                source,
            }
        })?)
    }
    fn from_connection(connection: Connection) -> Result<Self, StorageError> {
        connection
            .execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
            .map_err(|source| sql("configure SQLite", source))?;
        let mut store = Self { connection };
        store.migrate()?;
        Ok(store)
    }
    fn migrate(&mut self) -> Result<(), StorageError> {
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY NOT NULL, applied_at INTEGER NOT NULL DEFAULT (unixepoch()))")
            .map_err(|source| sql("create migration table", source))?;
        let current: u32 = self
            .connection
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                [],
                |row| row.get(0),
            )
            .map_err(|source| sql("read migration version", source))?;
        let latest = MIGRATIONS.last().map_or(0, |migration| migration.0);
        if current > latest {
            return Err(StorageError::NewerSchema { found: current });
        }
        for &(version, migration) in MIGRATIONS.iter().filter(|migration| migration.0 > current) {
            let tx = self
                .connection
                .transaction()
                .map_err(|source| StorageError::Migration { version, source })?;
            tx.execute_batch(migration)
                .map_err(|source| StorageError::Migration { version, source })?;
            tx.execute(
                "INSERT INTO schema_migrations(version) VALUES (?1)",
                params![version],
            )
            .map_err(|source| StorageError::Migration { version, source })?;
            tx.commit()
                .map_err(|source| StorageError::Migration { version, source })?;
        }
        Ok(())
    }
    pub fn migration_version(&self) -> Result<u32, StorageError> {
        self.connection
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                [],
                |row| row.get(0),
            )
            .map_err(|source| sql("read migration version", source))
    }
    pub fn connection(&self) -> &Connection {
        &self.connection
    }
    pub fn transaction<T>(
        &mut self,
        operation: impl FnOnce(&Transaction<'_>) -> Result<T, StorageError>,
    ) -> Result<T, StorageError> {
        let tx = self
            .connection
            .transaction()
            .map_err(|source| sql("begin transaction", source))?;
        let value = operation(&tx)?;
        tx.commit()
            .map_err(|source| sql("commit transaction", source))?;
        Ok(value)
    }

    pub fn save_account(&mut self, account: &Account) -> Result<(), StorageError> {
        domain("account", account.validate())?;
        self.transaction(|tx| {
            tx.execute("INSERT INTO accounts(id, display_name) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET display_name=excluded.display_name", params![account.id.as_str(), account.display_name])
                .map_err(|source| sql("upsert account", source))?;
            tx.execute("DELETE FROM devices WHERE account_id = ?1", params![account.id.as_str()]).map_err(|source| sql("replace devices", source))?;
            tx.execute("DELETE FROM identities WHERE account_id = ?1", params![account.id.as_str()]).map_err(|source| sql("replace identities", source))?;
            save_extensions(tx, "account", account.id.as_str(), &account.extensions)?;
            for identity in &account.identities {
                if let Some(person) = &identity.person_id {
                    tx.execute("INSERT OR IGNORE INTO persons(id, display_name) VALUES (?1, '')", params![person.as_str()]).map_err(|source| sql("create identity person", source))?;
                }
                let (kind, value, extra) = identity_parts(&identity.address);
                tx.execute("INSERT INTO identities(id, account_id, address_kind, address_value, address_extra, person_id, label) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![identity.id.as_str(), account.id.as_str(), kind, value, extra, identity.person_id.as_ref().map(PersonId::as_str), identity.label])
                    .map_err(|source| sql("insert identity", source))?;
                save_extensions(tx, "identity", identity.id.as_str(), &identity.extensions)?;
            }
            for device in &account.devices {
                tx.execute("INSERT INTO devices(id, account_id, identity_id, name, kind_json, capabilities_json, last_seen_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![device.id.as_str(), account.id.as_str(), device.identity_id.as_ref().map(IdentityId::as_str), device.name,
                        encode("device kind", &device.kind)?, encode("device capabilities", &device.capabilities)?, millis(device.last_seen_at)])
                    .map_err(|source| sql("insert device", source))?;
                save_extensions(tx, "device", device.id.as_str(), &device.extensions)?;
            }
            Ok(())
        })
    }

    pub fn get_account(&self, id_value: &AccountId) -> Result<Option<Account>, StorageError> {
        let display_name = self
            .connection
            .query_row(
                "SELECT display_name FROM accounts WHERE id = ?1",
                params![id_value.as_str()],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|source| sql("load account", source))?;
        let Some(display_name) = display_name else {
            return Ok(None);
        };
        let mut statement = self.connection.prepare("SELECT id, address_kind, address_value, address_extra, person_id, label FROM identities WHERE account_id = ?1 ORDER BY id").map_err(|source| sql("prepare identities", source))?;
        let rows = statement
            .query_map(params![id_value.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })
            .map_err(|source| sql("query identities", source))?;
        let mut identities = Vec::new();
        for row in rows {
            let (id_text, kind, value, extra, person, label) =
                row.map_err(|source| sql("read identity", source))?;
            let identity_id: IdentityId = id(id_text, "identity")?;
            identities.push(Identity {
                id: identity_id.clone(),
                address: identity_address(kind, value, extra)?,
                person_id: person.map(|value| id(value, "person")).transpose()?,
                label,
                extensions: load_extensions(&self.connection, "identity", identity_id.as_str())?,
            });
        }
        let mut statement = self.connection.prepare("SELECT id, identity_id, name, kind_json, capabilities_json, last_seen_at FROM devices WHERE account_id = ?1 ORDER BY id").map_err(|source| sql("prepare devices", source))?;
        let rows = statement
            .query_map(params![id_value.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                ))
            })
            .map_err(|source| sql("query devices", source))?;
        let mut devices = Vec::new();
        for row in rows {
            let (device_text, identity, name, kind, capabilities, last_seen_at) =
                row.map_err(|source| sql("read device", source))?;
            let device_id: DeviceId = id(device_text, "device")?;
            devices.push(Device {
                id: device_id.clone(),
                account_id: id_value.clone(),
                identity_id: identity.map(|value| id(value, "identity")).transpose()?,
                name,
                kind: decode("device kind", &kind)?,
                capabilities: decode("device capabilities", &capabilities)?,
                last_seen_at: timestamp(last_seen_at),
                extensions: load_extensions(&self.connection, "device", device_id.as_str())?,
            });
        }
        Ok(Some(Account {
            id: id_value.clone(),
            display_name,
            identities,
            devices,
            extensions: load_extensions(&self.connection, "account", id_value.as_str())?,
        }))
    }

    pub fn list_accounts(&self) -> Result<Vec<Account>, StorageError> {
        let ids = self
            .connection
            .prepare("SELECT id FROM accounts ORDER BY id")
            .map_err(|source| sql("prepare account list", source))?
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|source| sql("query account list", source))?
            .map(|row| {
                id(
                    row.map_err(|source| sql("read account id", source))?,
                    "account",
                )
            })
            .collect::<Result<Vec<AccountId>, _>>()?;
        ids.iter()
            .map(|id| {
                self.get_account(id)?
                    .ok_or_else(|| corrupt("account", "row disappeared while listing accounts"))
            })
            .collect()
    }

    pub fn save_conversation(&mut self, conversation: &Conversation) -> Result<(), StorageError> {
        domain("conversation", conversation.validate())?;
        self.transaction(|tx| {
            let kind = if matches!(conversation.kind, ConversationKind::Direct) { "direct" } else { "group" };
            tx.execute("INSERT INTO conversations(id, kind, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(id) DO UPDATE SET kind=excluded.kind, title=excluded.title, created_at=excluded.created_at, updated_at=excluded.updated_at", params![conversation.id.as_str(), kind, conversation.title, millis(conversation.created_at), millis(conversation.updated_at)]).map_err(|source| sql("upsert conversation", source))?;
            tx.execute("DELETE FROM group_metadata WHERE conversation_id = ?1", params![conversation.id.as_str()]).map_err(|source| sql("replace group metadata", source))?;
            tx.execute("DELETE FROM participants WHERE conversation_id = ?1", params![conversation.id.as_str()]).map_err(|source| sql("replace participants", source))?;
            save_extensions(tx, "conversation", conversation.id.as_str(), &conversation.extensions)?;
            for participant in &conversation.participants {
                if let Some(person) = &participant.person_id { tx.execute("INSERT OR IGNORE INTO persons(id, display_name) VALUES (?1, '')", params![person.as_str()]).map_err(|source| sql("create participant person", source))?; }
                tx.execute("INSERT INTO participants(id, conversation_id, person_id, identity_id, display_name, role_json, joined_at, left_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)", params![participant.id.as_str(), conversation.id.as_str(), participant.person_id.as_ref().map(PersonId::as_str), participant.identity_id.as_ref().map(IdentityId::as_str), participant.display_name, encode("participant role", &participant.role)?, millis(participant.joined_at), millis(participant.left_at)]).map_err(|source| sql("insert participant", source))?;
                save_extensions(tx, "participant", participant.id.as_str(), &participant.extensions)?;
            }
            if let ConversationKind::Group(details) = &conversation.kind {
                if let Some(avatar) = &details.avatar { save_attachment(tx, avatar)?; }
                tx.execute("INSERT INTO group_metadata(conversation_id, title, owner_participant_id, avatar_attachment_id) VALUES (?1, ?2, ?3, ?4)", params![conversation.id.as_str(), details.title, details.owner.as_ref().map(ParticipantId::as_str), details.avatar.as_ref().map(|avatar| avatar.id.as_str())]).map_err(|source| sql("insert group metadata", source))?;
            }
            Ok(())
        })
    }

    pub fn get_conversation(
        &self,
        id_value: &ConversationId,
    ) -> Result<Option<Conversation>, StorageError> {
        let row = self
            .connection
            .query_row(
                "SELECT kind, title, created_at, updated_at FROM conversations WHERE id = ?1",
                params![id_value.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|source| sql("load conversation", source))?;
        let Some((kind, title, created_at, updated_at)) = row else {
            return Ok(None);
        };
        let mut statement = self.connection.prepare("SELECT id, person_id, identity_id, display_name, role_json, joined_at, left_at FROM participants WHERE conversation_id = ?1 ORDER BY id").map_err(|source| sql("prepare participants", source))?;
        let rows = statement
            .query_map(params![id_value.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                ))
            })
            .map_err(|source| sql("query participants", source))?;
        let mut participants = Vec::new();
        for row in rows {
            let (participant_text, person, identity, display_name, role, joined_at, left_at) =
                row.map_err(|source| sql("read participant", source))?;
            let participant_id: ParticipantId = id(participant_text, "participant")?;
            participants.push(Participant {
                id: participant_id.clone(),
                person_id: person.map(|value| id(value, "person")).transpose()?,
                identity_id: identity.map(|value| id(value, "identity")).transpose()?,
                display_name,
                role: decode("participant role", &role)?,
                joined_at: timestamp(joined_at),
                left_at: timestamp(left_at),
                extensions: load_extensions(
                    &self.connection,
                    "participant",
                    participant_id.as_str(),
                )?,
            });
        }
        let group = self.connection.query_row("SELECT title, owner_participant_id, avatar_attachment_id FROM group_metadata WHERE conversation_id = ?1", params![id_value.as_str()], |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?))).optional().map_err(|source| sql("load group metadata", source))?;
        let kind = match kind.as_str() {
            "direct" => ConversationKind::Direct,
            "group" => {
                let Some((group_title, owner, avatar)) = group else {
                    return Err(corrupt("conversation", "group has no metadata"));
                };
                ConversationKind::Group(Box::new(litebubbles_core::GroupDetails {
                    title: group_title,
                    owner: owner.map(|value| id(value, "participant")).transpose()?,
                    avatar: avatar
                        .map(|value| load_attachment(&self.connection, &value))
                        .transpose()?
                        .flatten(),
                }))
            }
            _ => return Err(corrupt("conversation", "unknown kind")),
        };
        Ok(Some(Conversation {
            id: id_value.clone(),
            kind,
            title,
            participants,
            created_at: timestamp(created_at),
            updated_at: timestamp(updated_at),
            extensions: load_extensions(&self.connection, "conversation", id_value.as_str())?,
        }))
    }

    pub fn list_conversations(&self) -> Result<Vec<Conversation>, StorageError> {
        let ids = self
            .connection
            .prepare(
                "SELECT id FROM conversations ORDER BY COALESCE(updated_at, created_at, 0), id",
            )
            .map_err(|source| sql("prepare conversation list", source))?
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|source| sql("query conversation list", source))?
            .map(|row| {
                id(
                    row.map_err(|source| sql("read conversation id", source))?,
                    "conversation",
                )
            })
            .collect::<Result<Vec<ConversationId>, _>>()?;
        ids.iter()
            .map(|id| {
                self.get_conversation(id)?.ok_or_else(|| {
                    corrupt(
                        "conversation",
                        "row disappeared while listing conversations",
                    )
                })
            })
            .collect()
    }

    pub fn save_message(&mut self, message: &Message) -> Result<(), StorageError> {
        domain("message", message.validate())?;
        self.transaction(|tx| {
            let (read_state, read_at) = match message.read_state { ReadState::Unread => ("unread", None), ReadState::Read { at } => ("read", Some(at.as_millis())) };
            tx.execute("INSERT INTO messages(id, conversation_id, sender_id, sent_at, delivery_json, read_state, read_at, reply_to_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) ON CONFLICT(id) DO UPDATE SET conversation_id=excluded.conversation_id, sender_id=excluded.sender_id, sent_at=excluded.sent_at, delivery_json=excluded.delivery_json, read_state=excluded.read_state, read_at=excluded.read_at, reply_to_id=excluded.reply_to_id",
                params![message.id.as_str(), message.conversation_id.as_str(), message.sender.as_ref().map(ParticipantId::as_str), message.sent_at.as_millis(), encode("delivery state", &message.delivery)?, read_state, read_at, message.reply_to.as_ref().map(MessageId::as_str)])
                .map_err(|source| sql("upsert message", source))?;
            for table in ["message_parts", "message_mutations", "reactions", "delivery_receipts", "read_receipts"] {
                tx.execute(&format!("DELETE FROM {table} WHERE message_id = ?1"), params![message.id.as_str()]).map_err(|source| sql("replace message children", source))?;
            }
            save_extensions(tx, "message", message.id.as_str(), &message.extensions)?;
            for (index, part) in message.parts.iter().enumerate() {
                save_part_attachments(tx, part)?;
                tx.execute("INSERT INTO message_parts(message_id, part_index, part_kind, payload_json) VALUES (?1, ?2, ?3, ?4)", params![message.id.as_str(), i64::try_from(index).map_err(|_| corrupt("message part", "too many parts"))?, part_kind(part), encode("message part", part)?]).map_err(|source| sql("insert message part", source))?;
            }
            for mutation in &message.mutations {
                if let litebubbles_core::MessageMutationKind::Edit { parts } = &mutation.kind { for part in parts { save_part_attachments(tx, part)?; } }
                tx.execute("INSERT INTO message_mutations(id, message_id, actor_id, occurred_at, kind_json) VALUES (?1, ?2, ?3, ?4, ?5)", params![mutation.id.as_str(), message.id.as_str(), mutation.actor.as_ref().map(ParticipantId::as_str), mutation.occurred_at.as_millis(), encode("message mutation", &mutation.kind)?]).map_err(|source| sql("insert message mutation", source))?;
            }
            for reaction in &message.reactions {
                tx.execute("INSERT INTO reactions(id, message_id, participant_id, kind_json, created_at, removed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![reaction.id.as_str(), message.id.as_str(), reaction.participant_id.as_str(), encode("reaction kind", &reaction.kind)?, reaction.created_at.as_millis(), millis(reaction.removed_at)]).map_err(|source| sql("insert reaction", source))?;
            }
            for receipt in &message.delivery_receipts {
                tx.execute("INSERT INTO delivery_receipts(message_id, participant_id, state_json, at) VALUES (?1, ?2, ?3, ?4)", params![message.id.as_str(), receipt.participant_id.as_str(), encode("delivery receipt", &receipt.state)?, millis(receipt.at)]).map_err(|source| sql("insert delivery receipt", source))?;
            }
            for receipt in &message.read_receipts {
                tx.execute("INSERT INTO read_receipts(message_id, participant_id, at) VALUES (?1, ?2, ?3)", params![message.id.as_str(), receipt.participant_id.as_str(), receipt.at.as_millis()]).map_err(|source| sql("insert read receipt", source))?;
            }
            Ok(())
        })
    }

    pub fn get_message(&self, id_value: &MessageId) -> Result<Option<Message>, StorageError> {
        let row = self.connection.query_row("SELECT conversation_id, sender_id, sent_at, delivery_json, read_state, read_at, reply_to_id FROM messages WHERE id = ?1", params![id_value.as_str()], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, i64>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, Option<i64>>(5)?, row.get::<_, Option<String>>(6)?))).optional().map_err(|source| sql("load message", source))?;
        let Some((conversation, sender, sent_at, delivery, read_state, read_at, reply_to)) = row
        else {
            return Ok(None);
        };
        let conversation_id: ConversationId = id(conversation, "conversation")?;
        let sender = sender.map(|value| id(value, "participant")).transpose()?;
        let mut parts_statement = self
            .connection
            .prepare(
                "SELECT payload_json FROM message_parts WHERE message_id = ?1 ORDER BY part_index",
            )
            .map_err(|source| sql("prepare message parts", source))?;
        let parts = parts_statement
            .query_map(params![id_value.as_str()], |row| row.get::<_, String>(0))
            .map_err(|source| sql("query message parts", source))?
            .map(|row| {
                decode(
                    "message part",
                    &row.map_err(|source| sql("read message part", source))?,
                )
            })
            .collect::<Result<Vec<MessagePart>, _>>()?;
        let mut mutation_statement = self.connection.prepare("SELECT id, actor_id, occurred_at, kind_json FROM message_mutations WHERE message_id = ?1 ORDER BY occurred_at, id").map_err(|source| sql("prepare message mutations", source))?;
        let mutations = mutation_statement
            .query_map(params![id_value.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|source| sql("query message mutations", source))?
            .map(|row| {
                let (mutation, actor, occurred_at, kind) =
                    row.map_err(|source| sql("read message mutation", source))?;
                Ok(MessageMutation {
                    id: id(mutation, "message mutation")?,
                    message_id: id_value.clone(),
                    actor: actor.map(|value| id(value, "participant")).transpose()?,
                    occurred_at: Timestamp::from_millis(occurred_at),
                    kind: decode("message mutation", &kind)?,
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        let mut reaction_statement = self.connection.prepare("SELECT id, participant_id, kind_json, created_at, removed_at FROM reactions WHERE message_id = ?1 ORDER BY created_at, id").map_err(|source| sql("prepare reactions", source))?;
        let reactions = reaction_statement
            .query_map(params![id_value.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                ))
            })
            .map_err(|source| sql("query reactions", source))?
            .map(|row| {
                let (reaction, participant, kind, created_at, removed_at) =
                    row.map_err(|source| sql("read reaction", source))?;
                Ok(Reaction {
                    id: id(reaction, "reaction")?,
                    message_id: id_value.clone(),
                    participant_id: id(participant, "participant")?,
                    kind: decode("reaction kind", &kind)?,
                    created_at: Timestamp::from_millis(created_at),
                    removed_at: timestamp(removed_at),
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        let delivery_receipts = self.load_delivery_receipts(id_value)?;
        let read_receipts = self.load_read_receipts(id_value)?;
        let read_state = match read_state.as_str() {
            "unread" => ReadState::Unread,
            "read" => ReadState::Read {
                at: Timestamp::from_millis(
                    read_at.ok_or_else(|| corrupt("read state", "missing timestamp"))?,
                ),
            },
            _ => return Err(corrupt("read state", "unknown state")),
        };
        Ok(Some(Message {
            id: id_value.clone(),
            conversation_id,
            sender,
            sent_at: Timestamp::from_millis(sent_at),
            parts,
            mutations,
            reactions,
            delivery: decode("delivery state", &delivery)?,
            delivery_receipts,
            read_state,
            read_receipts,
            reply_to: reply_to
                .map(|value| id(value, "reply message"))
                .transpose()?,
            extensions: load_extensions(&self.connection, "message", id_value.as_str())?,
        }))
    }

    fn load_delivery_receipts(
        &self,
        message_id: &MessageId,
    ) -> Result<Vec<DeliveryReceipt>, StorageError> {
        let mut statement = self.connection.prepare("SELECT participant_id, state_json, at FROM delivery_receipts WHERE message_id = ?1 ORDER BY participant_id").map_err(|source| sql("prepare delivery receipts", source))?;
        statement
            .query_map(params![message_id.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            })
            .map_err(|source| sql("query delivery receipts", source))?
            .map(|row| {
                let (participant, state, at) =
                    row.map_err(|source| sql("read delivery receipt", source))?;
                Ok(DeliveryReceipt {
                    participant_id: id(participant, "participant")?,
                    state: decode("delivery receipt", &state)?,
                    at: timestamp(at),
                })
            })
            .collect()
    }
    fn load_read_receipts(&self, message_id: &MessageId) -> Result<Vec<ReadReceipt>, StorageError> {
        let mut statement = self.connection.prepare("SELECT participant_id, at FROM read_receipts WHERE message_id = ?1 ORDER BY participant_id").map_err(|source| sql("prepare read receipts", source))?;
        statement
            .query_map(params![message_id.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|source| sql("query read receipts", source))?
            .map(|row| {
                let (participant, at) = row.map_err(|source| sql("read read receipt", source))?;
                Ok(ReadReceipt {
                    participant_id: id(participant, "participant")?,
                    at: Timestamp::from_millis(at),
                })
            })
            .collect()
    }

    pub fn list_messages(
        &self,
        conversation_id: &ConversationId,
        page: MessagePage,
    ) -> Result<Vec<Message>, StorageError> {
        if page.limit == 0 || (page.after.is_some() && page.before.is_some()) {
            return Err(StorageError::InvalidPageLimit);
        }
        let mut ids = Vec::new();
        if let Some(cursor) = page.after {
            let mut statement = self.connection.prepare("SELECT id FROM messages WHERE conversation_id = ?1 AND (sent_at > ?2 OR (sent_at = ?2 AND id > ?3)) ORDER BY sent_at, id LIMIT ?4").map_err(|source| sql("prepare message page", source))?;
            let rows = statement
                .query_map(
                    params![
                        conversation_id.as_str(),
                        cursor.sent_at.as_millis(),
                        cursor.id.as_str(),
                        page.limit
                    ],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|source| sql("query message page", source))?;
            for row in rows {
                ids.push(id(
                    row.map_err(|source| sql("read message page", source))?,
                    "message",
                )?);
            }
        } else if let Some(cursor) = page.before {
            let mut statement = self.connection.prepare("SELECT id FROM messages WHERE conversation_id = ?1 AND (sent_at < ?2 OR (sent_at = ?2 AND id < ?3)) ORDER BY sent_at DESC, id DESC LIMIT ?4").map_err(|source| sql("prepare message page", source))?;
            let rows = statement
                .query_map(
                    params![
                        conversation_id.as_str(),
                        cursor.sent_at.as_millis(),
                        cursor.id.as_str(),
                        page.limit
                    ],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|source| sql("query message page", source))?;
            for row in rows {
                ids.push(id(
                    row.map_err(|source| sql("read message page", source))?,
                    "message",
                )?);
            }
            ids.reverse();
        } else {
            let mut statement = self.connection.prepare("SELECT id FROM messages WHERE conversation_id = ?1 ORDER BY sent_at, id LIMIT ?2").map_err(|source| sql("prepare message page", source))?;
            let rows = statement
                .query_map(params![conversation_id.as_str(), page.limit], |row| {
                    row.get::<_, String>(0)
                })
                .map_err(|source| sql("query message page", source))?;
            for row in rows {
                ids.push(id(
                    row.map_err(|source| sql("read message page", source))?,
                    "message",
                )?);
            }
        }
        ids.iter()
            .map(|id| {
                self.get_message(id)?
                    .ok_or_else(|| corrupt("message", "row disappeared while listing messages"))
            })
            .collect()
    }

    pub fn save_sync_metadata(&mut self, value: &SyncMetadata) -> Result<(), StorageError> {
        self.transaction(|tx| {
            tx.execute("INSERT INTO sync_metadata(service, account_id, cursor, last_synced_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(service, account_id) DO UPDATE SET cursor=excluded.cursor, last_synced_at=excluded.last_synced_at, updated_at=excluded.updated_at", params![value.service, account_key(value.account_id.as_ref()), value.cursor, millis(value.last_synced_at), value.updated_at.as_millis()]).map_err(|source| sql("save sync metadata", source))?;
            Ok(())
        })
    }
    pub fn sync_metadata(
        &self,
        service: &str,
        account_id: Option<&AccountId>,
    ) -> Result<Option<SyncMetadata>, StorageError> {
        self.connection.query_row("SELECT cursor, last_synced_at, updated_at FROM sync_metadata WHERE service = ?1 AND account_id = ?2", params![service, account_key(account_id)], |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<i64>>(1)?, row.get::<_, i64>(2)?))).optional().map_err(|source| sql("load sync metadata", source))?.map(|(cursor, last_synced_at, updated_at)| Ok(SyncMetadata { service: service.to_owned(), account_id: account_id.cloned(), cursor, last_synced_at: timestamp(last_synced_at), updated_at: Timestamp::from_millis(updated_at) })).transpose()
    }
    pub fn save_service_sync_state(
        &mut self,
        value: &ServiceSyncState,
    ) -> Result<(), StorageError> {
        self.transaction(|tx| {
            tx.execute("INSERT INTO service_sync_state(service, account_id, state_key, value, updated_at) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(service, account_id, state_key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at", params![value.service, account_key(value.account_id.as_ref()), value.key, value.value, value.updated_at.as_millis()]).map_err(|source| sql("save service sync state", source))?;
            Ok(())
        })
    }
    pub fn service_sync_state(
        &self,
        service: &str,
        account_id: Option<&AccountId>,
        key: &str,
    ) -> Result<Option<ServiceSyncState>, StorageError> {
        self.connection.query_row("SELECT value, updated_at FROM service_sync_state WHERE service = ?1 AND account_id = ?2 AND state_key = ?3", params![service, account_key(account_id), key], |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?))).optional().map_err(|source| sql("load service sync state", source))?.map(|(value, updated_at)| Ok(ServiceSyncState { service: service.to_owned(), account_id: account_id.cloned(), key: key.to_owned(), value, updated_at: Timestamp::from_millis(updated_at) })).transpose()
    }
    pub fn set_unread_state(&mut self, value: &UnreadState) -> Result<(), StorageError> {
        let count = i64::try_from(value.unread_count)
            .map_err(|_| corrupt("unread state", "count exceeds SQLite integer range"))?;
        self.transaction(|tx| {
            tx.execute("INSERT INTO unread_state(conversation_id, participant_id, unread_count, last_read_at) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(conversation_id) DO UPDATE SET participant_id=excluded.participant_id, unread_count=excluded.unread_count, last_read_at=excluded.last_read_at", params![value.conversation_id.as_str(), value.participant_id.as_ref().map(ParticipantId::as_str), count, millis(value.last_read_at)]).map_err(|source| sql("save unread state", source))?;
            Ok(())
        })
    }
    pub fn unread_state(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Option<UnreadState>, StorageError> {
        self.connection.query_row("SELECT participant_id, unread_count, last_read_at FROM unread_state WHERE conversation_id = ?1", params![conversation_id.as_str()], |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?, row.get::<_, Option<i64>>(2)?))).optional().map_err(|source| sql("load unread state", source))?.map(|(participant, count, last_read_at)| Ok(UnreadState { conversation_id: conversation_id.clone(), participant_id: participant.map(|value| id(value, "participant")).transpose()?, unread_count: u64::try_from(count).map_err(|_| corrupt("unread state", "negative count"))?, last_read_at: timestamp(last_read_at) })).transpose()
    }
    pub fn claim_deduplication_key(
        &mut self,
        scope: &str,
        key: &str,
        object_kind: &str,
        object_id: &str,
        created_at: Timestamp,
    ) -> Result<bool, StorageError> {
        self.transaction(|tx| {
            let inserted = tx.execute("INSERT OR IGNORE INTO deduplication_keys(scope, deduplication_key, object_kind, object_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5)", params![scope, key, object_kind, object_id, created_at.as_millis()]).map_err(|source| sql("claim deduplication key", source))?;
            Ok(inserted == 1)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use litebubbles_core::{DeliveryState, GroupDetails, ParticipantRole, ReactionKind, TextPart};
    use tempfile::NamedTempFile;

    fn id<T>(value: &str) -> T
    where
        T: TryFrom<String, Error = litebubbles_core::IdError>,
    {
        T::try_from(value.to_owned()).expect("valid test id")
    }
    fn account() -> Account {
        Account {
            id: id("account"),
            display_name: Some("Primary".to_owned()),
            identities: vec![Identity {
                id: id("identity"),
                address: IdentityAddress::Email("person@example.test".to_owned()),
                person_id: None,
                label: Some("work".to_owned()),
                extensions: vec![],
            }],
            devices: vec![],
            extensions: vec![],
        }
    }
    fn conversation() -> Conversation {
        let participant = Participant {
            id: id("participant"),
            person_id: None,
            identity_id: Some(id("identity")),
            display_name: Some("Person".to_owned()),
            role: ParticipantRole::Owner,
            joined_at: None,
            left_at: None,
            extensions: vec![],
        };
        Conversation {
            id: id("conversation"),
            kind: ConversationKind::Group(Box::new(GroupDetails {
                title: Some("Group".to_owned()),
                owner: Some(participant.id.clone()),
                avatar: None,
            })),
            title: Some("Group".to_owned()),
            participants: vec![participant],
            created_at: Some(Timestamp::from_millis(1)),
            updated_at: Some(Timestamp::from_millis(2)),
            extensions: vec![],
        }
    }
    fn message(value: &str, sent_at: i64) -> Message {
        Message {
            id: id(value),
            conversation_id: id("conversation"),
            sender: Some(id("participant")),
            sent_at: Timestamp::from_millis(sent_at),
            parts: vec![MessagePart::Text(TextPart {
                text: value.to_owned(),
                formatting: vec![],
            })],
            mutations: vec![],
            reactions: vec![Reaction {
                id: id(&format!("reaction-{value}")),
                message_id: id(value),
                participant_id: id("participant"),
                kind: ReactionKind::Like,
                created_at: Timestamp::from_millis(sent_at + 1),
                removed_at: None,
            }],
            delivery: DeliveryState::Sent,
            delivery_receipts: vec![],
            read_state: ReadState::Unread,
            read_receipts: vec![],
            reply_to: None,
            extensions: vec![],
        }
    }

    #[test]
    fn fresh_database_migrates_and_creates_required_indexes() {
        let store = Store::open_in_memory().expect("open");
        assert_eq!(store.migration_version().expect("version"), 2);
        let tables: i64 = store.connection().query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('accounts','identities','conversations','participants','messages','message_parts','reactions','attachments','sync_metadata','unread_state','service_sync_state')", [], |row| row.get(0)).expect("tables");
        assert_eq!(tables, 11);
        let indexes: i64 = store.connection().query_row("SELECT count(*) FROM sqlite_master WHERE type='index' AND name='idx_messages_conversation_order'", [], |row| row.get(0)).expect("index");
        assert_eq!(indexes, 1);
    }
    #[test]
    fn file_migrations_are_repeatable() {
        let file = NamedTempFile::new().expect("temp db");
        let store = Store::open_path(file.path()).expect("first open");
        assert_eq!(store.migration_version().expect("version"), 2);
        drop(store);
        assert_eq!(
            Store::open_path(file.path())
                .expect("second open")
                .migration_version()
                .expect("version"),
            2
        );
    }
    #[test]
    fn failed_transaction_rolls_back() {
        let mut store = Store::open_in_memory().expect("open");
        let result: Result<(), StorageError> = store.transaction(|tx| {
            tx.execute("INSERT INTO accounts(id) VALUES ('temporary')", [])
                .map_err(|source| sql("test insert", source))?;
            Err(StorageError::Corrupt {
                entity: "test",
                message: "rollback".to_owned(),
            })
        });
        assert!(result.is_err());
        assert_eq!(
            store
                .connection()
                .query_row("SELECT count(*) FROM accounts", [], |row| row
                    .get::<_, i64>(0))
                .expect("count"),
            0
        );
    }
    #[test]
    fn repositories_round_trip_domain_values() {
        let mut store = Store::open_in_memory().expect("open");
        let account = account();
        store.save_account(&account).expect("account");
        assert_eq!(store.get_account(&account.id).expect("load"), Some(account));
        let conversation = conversation();
        store
            .save_conversation(&conversation)
            .expect("conversation");
        assert_eq!(
            store.get_conversation(&conversation.id).expect("load"),
            Some(conversation)
        );
        let message = message("message", 10);
        store.save_message(&message).expect("message");
        assert_eq!(store.get_message(&message.id).expect("load"), Some(message));
    }
    #[test]
    fn pagination_is_deterministic_and_deduplication_is_idempotent() {
        let mut store = Store::open_in_memory().expect("open");
        store
            .save_conversation(&conversation())
            .expect("conversation");
        for (value, time) in [("b", 10), ("a", 10), ("c", 11)] {
            store.save_message(&message(value, time)).expect("message");
        }
        let first = store
            .list_messages(&id("conversation"), MessagePage::new(2).expect("page"))
            .expect("page");
        assert_eq!(
            first
                .iter()
                .map(|value| value.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        let cursor = MessageCursor {
            sent_at: first[1].sent_at,
            id: first[1].id.clone(),
        };
        let second = store
            .list_messages(
                &id("conversation"),
                MessagePage::new(2).expect("page").after(cursor),
            )
            .expect("page");
        assert_eq!(second[0].id.as_str(), "c");
        assert!(
            store
                .claim_deduplication_key(
                    "service",
                    "event",
                    "message",
                    "a",
                    Timestamp::from_millis(1)
                )
                .expect("claim")
        );
        assert!(
            !store
                .claim_deduplication_key(
                    "service",
                    "event",
                    "message",
                    "a",
                    Timestamp::from_millis(2)
                )
                .expect("duplicate")
        );
    }
    #[test]
    fn sync_and_unread_state_are_scoped() {
        let mut store = Store::open_in_memory().expect("open");
        let account_id: AccountId = id("account");
        store
            .save_sync_metadata(&SyncMetadata {
                service: "messages".to_owned(),
                account_id: Some(account_id.clone()),
                cursor: Some("cursor".to_owned()),
                last_synced_at: None,
                updated_at: Timestamp::from_millis(1),
            })
            .expect("sync");
        assert_eq!(
            store
                .sync_metadata("messages", Some(&account_id))
                .expect("load")
                .expect("value")
                .cursor
                .as_deref(),
            Some("cursor")
        );
        store
            .save_service_sync_state(&ServiceSyncState {
                service: "messages".to_owned(),
                account_id: Some(account_id.clone()),
                key: "checkpoint".to_owned(),
                value: vec![1, 2, 3],
                updated_at: Timestamp::from_millis(2),
            })
            .expect("service state");
        assert_eq!(
            store
                .service_sync_state("messages", Some(&account_id), "checkpoint")
                .expect("load service state")
                .expect("service state")
                .value,
            vec![1, 2, 3]
        );
        store
            .save_conversation(&conversation())
            .expect("conversation");
        store
            .set_unread_state(&UnreadState {
                conversation_id: id("conversation"),
                participant_id: Some(id("participant")),
                unread_count: 3,
                last_read_at: None,
            })
            .expect("unread");
        assert_eq!(
            store
                .unread_state(&id("conversation"))
                .expect("load")
                .expect("state")
                .unread_count,
            3
        );
    }
}
