CREATE TABLE accounts (
    id TEXT PRIMARY KEY NOT NULL,
    display_name TEXT
);

CREATE TABLE persons (
    id TEXT PRIMARY KEY NOT NULL,
    display_name TEXT NOT NULL
);

CREATE TABLE identities (
    id TEXT PRIMARY KEY NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    address_kind TEXT NOT NULL,
    address_value TEXT NOT NULL,
    address_extra TEXT,
    person_id TEXT REFERENCES persons(id) ON DELETE SET NULL,
    label TEXT,
    UNIQUE(account_id, address_kind, address_value, address_extra)
);

CREATE TABLE devices (
    id TEXT PRIMARY KEY NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    identity_id TEXT REFERENCES identities(id) ON DELETE SET NULL,
    name TEXT,
    kind_json TEXT NOT NULL,
    capabilities_json TEXT NOT NULL,
    last_seen_at INTEGER
);

CREATE TABLE conversations (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('direct', 'group')),
    title TEXT,
    created_at INTEGER,
    updated_at INTEGER
);

CREATE TABLE participants (
    id TEXT PRIMARY KEY NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    person_id TEXT REFERENCES persons(id) ON DELETE SET NULL,
    identity_id TEXT,
    display_name TEXT,
    role_json TEXT NOT NULL,
    joined_at INTEGER,
    left_at INTEGER,
    UNIQUE(conversation_id, id)
);

CREATE TABLE group_metadata (
    conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
    title TEXT,
    owner_participant_id TEXT REFERENCES participants(id) ON DELETE SET NULL,
    avatar_attachment_id TEXT
);

CREATE TABLE attachments (
    id TEXT PRIMARY KEY NOT NULL,
    kind_json TEXT NOT NULL,
    file_name TEXT,
    mime_type TEXT,
    byte_size INTEGER,
    content_kind TEXT NOT NULL CHECK (content_kind IN ('inline', 'external')),
    content BLOB NOT NULL
);

CREATE TABLE messages (
    id TEXT PRIMARY KEY NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    sender_id TEXT,
    sent_at INTEGER NOT NULL,
    delivery_json TEXT NOT NULL,
    read_state TEXT NOT NULL CHECK (read_state IN ('unread', 'read')),
    read_at INTEGER,
    reply_to_id TEXT REFERENCES messages(id) ON DELETE SET NULL,
    UNIQUE(conversation_id, id)
);

CREATE TABLE message_parts (
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    part_index INTEGER NOT NULL CHECK (part_index >= 0),
    part_kind TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    PRIMARY KEY(message_id, part_index)
);

CREATE TABLE message_mutations (
    id TEXT PRIMARY KEY NOT NULL,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    actor_id TEXT,
    occurred_at INTEGER NOT NULL,
    kind_json TEXT NOT NULL
);

CREATE TABLE reactions (
    id TEXT PRIMARY KEY NOT NULL,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    participant_id TEXT NOT NULL,
    kind_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    removed_at INTEGER
);

CREATE TABLE delivery_receipts (
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    participant_id TEXT NOT NULL,
    state_json TEXT NOT NULL,
    at INTEGER,
    PRIMARY KEY(message_id, participant_id)
);

CREATE TABLE read_receipts (
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    participant_id TEXT NOT NULL,
    at INTEGER NOT NULL,
    PRIMARY KEY(message_id, participant_id)
);

CREATE TABLE unread_state (
    conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
    participant_id TEXT,
    unread_count INTEGER NOT NULL CHECK (unread_count >= 0),
    last_read_at INTEGER
);

CREATE TABLE extensions (
    owner_kind TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    service TEXT NOT NULL,
    name TEXT NOT NULL,
    content_type TEXT,
    payload BLOB NOT NULL,
    PRIMARY KEY(owner_kind, owner_id, ordinal)
);

CREATE TABLE sync_metadata (
    service TEXT NOT NULL,
    account_id TEXT NOT NULL DEFAULT '',
    cursor TEXT,
    last_synced_at INTEGER,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(service, account_id)
);

CREATE TABLE service_sync_state (
    service TEXT NOT NULL,
    account_id TEXT NOT NULL DEFAULT '',
    state_key TEXT NOT NULL,
    value BLOB NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(service, account_id, state_key)
);

CREATE TABLE deduplication_keys (
    scope TEXT NOT NULL,
    deduplication_key TEXT NOT NULL,
    object_kind TEXT NOT NULL,
    object_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY(scope, deduplication_key)
);
