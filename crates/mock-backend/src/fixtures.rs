//! Synthetic core-domain snapshots and live-event scripts.

use litebubbles_core::{
    Account, AccountId, AlbumAssetKind, Attachment, AttachmentContent, AttachmentId,
    AttachmentKind, BackendEvent, BackendEventId, BackendEventKind, Call, CallId, CallKind,
    CallParticipant, CallParticipantState, CallState, ContactCard, Conversation, ConversationId,
    ConversationKind, DeliveryReceipt, DeliveryState, Device, DeviceCapability, DeviceId,
    DeviceKind, FindMyItem, FindMyItemId, FindMyItemKind, FindMyItemStatus, GeoPoint, GroupDetails,
    IdError, Identity, IdentityAddress, IdentityId, Location, Message, MessageId, MessageMutation,
    MessageMutationId, MessageMutationKind, MessagePart, Participant, ParticipantId,
    ParticipantRole, Person, PersonId, Reaction, ReactionId, ReactionKind, ReadReceipt, ReadState,
    ServiceExtension, SharedAlbum, SharedAlbumAsset, SharedAlbumAssetId, SharedAlbumId,
    SharedAlbumMember, TextFormatting, TextPart, TextRange, TextStyle, Timestamp, TypingIndicator,
    TypingState,
};

use crate::{EventScript, FixtureSet};

/// Fixed epoch used by every built-in fixture.
pub const FIXTURE_EPOCH_MS: i64 = 1_700_000_000_000;

/// Builds the standard synthetic state used by [`crate::MockBackend`].
pub fn representative() -> FixtureSet {
    let account_id = id::<AccountId>("account-fixture");
    let identity = Identity {
        id: id::<IdentityId>("identity-alex-fixture"),
        address: IdentityAddress::Email("alex@fixture.test".to_owned()),
        person_id: Some(id::<PersonId>("person-alex-fixture")),
        label: Some("Primary fixture identity".to_owned()),
        extensions: vec![extension(
            "fixture.identity",
            "identity-kind",
            "synthetic-email",
        )],
    };

    let avatar_maya = Attachment {
        id: id::<AttachmentId>("attachment-avatar-maya"),
        kind: AttachmentKind::Image,
        file_name: Some("maya-avatar.fixture".to_owned()),
        mime_type: Some("image/x-fixture".to_owned()),
        byte_size: Some(14),
        content: AttachmentContent::Inline(b"fixture-avatar".to_vec()),
        extensions: vec![extension("fixture.media", "generated", "true")],
    };
    let group_avatar = Attachment {
        id: id::<AttachmentId>("attachment-avatar-group"),
        kind: AttachmentKind::Image,
        file_name: Some("weekend-group.fixture".to_owned()),
        mime_type: Some("image/x-fixture".to_owned()),
        byte_size: Some(13),
        content: AttachmentContent::Inline(b"fixture-group".to_vec()),
        extensions: vec![],
    };
    let garden_photo = Attachment {
        id: id::<AttachmentId>("attachment-garden-photo"),
        kind: AttachmentKind::Image,
        file_name: Some("garden-photo.fixture".to_owned()),
        mime_type: Some("image/x-fixture".to_owned()),
        byte_size: Some(12),
        content: AttachmentContent::Inline(b"fixture-photo".to_vec()),
        extensions: vec![extension("fixture.media", "thumbnail", "available")],
    };
    let voice_note = Attachment {
        id: id::<AttachmentId>("attachment-voice-note"),
        kind: AttachmentKind::Audio,
        file_name: Some("voice-note.fixture".to_owned()),
        mime_type: Some("audio/x-fixture".to_owned()),
        byte_size: Some(18),
        content: AttachmentContent::External {
            token: "fixture://attachments/voice-note".to_owned(),
        },
        extensions: vec![],
    };
    let map_file = Attachment {
        id: id::<AttachmentId>("attachment-trip-map"),
        kind: AttachmentKind::File,
        file_name: Some("trip-map.fixture".to_owned()),
        mime_type: Some("application/x-fixture".to_owned()),
        byte_size: Some(16),
        content: AttachmentContent::External {
            token: "fixture://attachments/trip-map".to_owned(),
        },
        extensions: vec![],
    };
    let album_photo = Attachment {
        id: id::<AttachmentId>("attachment-album-photo"),
        kind: AttachmentKind::Image,
        file_name: Some("album-photo.fixture".to_owned()),
        mime_type: Some("image/x-fixture".to_owned()),
        byte_size: Some(12),
        content: AttachmentContent::Inline(b"fixture-album".to_vec()),
        extensions: vec![],
    };

    let people = vec![
        Person {
            id: id::<PersonId>("person-alex-fixture"),
            display_name: "Alex Fixture".to_owned(),
            identity_ids: vec![identity.id.clone()],
            avatar: None,
            extensions: vec![extension("fixture.person", "role", "account")],
        },
        Person {
            id: id::<PersonId>("person-maya-fixture"),
            display_name: "Maya Fixture".to_owned(),
            identity_ids: vec![id::<IdentityId>("identity-maya-fixture")],
            avatar: Some(avatar_maya.clone()),
            extensions: vec![],
        },
        Person {
            id: id::<PersonId>("person-jordan-fixture"),
            display_name: "Jordan Fixture".to_owned(),
            identity_ids: vec![id::<IdentityId>("identity-jordan-fixture")],
            avatar: None,
            extensions: vec![],
        },
        Person {
            id: id::<PersonId>("person-devon-fixture"),
            display_name: "Devon Fixture".to_owned(),
            identity_ids: vec![id::<IdentityId>("identity-devon-fixture")],
            avatar: None,
            extensions: vec![],
        },
    ];

    let devices = vec![
        Device {
            id: id::<DeviceId>("device-fixture-phone"),
            account_id: account_id.clone(),
            identity_id: Some(identity.id.clone()),
            name: Some("Fixture Phone".to_owned()),
            kind: DeviceKind::Phone,
            capabilities: vec![DeviceCapability::Messaging, DeviceCapability::AudioCalls],
            last_seen_at: Some(ts(2_000)),
            extensions: vec![],
        },
        Device {
            id: id::<DeviceId>("device-fixture-computer"),
            account_id: account_id.clone(),
            identity_id: Some(identity.id.clone()),
            name: Some("Fixture Computer".to_owned()),
            kind: DeviceKind::Computer,
            capabilities: vec![
                DeviceCapability::Messaging,
                DeviceCapability::VideoCalls,
                DeviceCapability::FindMy,
                DeviceCapability::SharedAlbums,
            ],
            last_seen_at: Some(ts(3_000)),
            extensions: vec![extension("fixture.device", "desktop-mode", "enabled")],
        },
    ];

    let account = Account {
        id: account_id.clone(),
        display_name: Some("Alex Fixture".to_owned()),
        identities: vec![identity.clone()],
        devices,
        extensions: vec![extension("fixture.account", "account-kind", "synthetic")],
    };

    let participant_alex_direct = participant(
        "participant-alex-direct",
        "person-alex-fixture",
        Some(identity.id.clone()),
        "Alex Fixture",
        ParticipantRole::Member,
        -86_400_000,
    );
    let participant_maya_direct = participant(
        "participant-maya-direct",
        "person-maya-fixture",
        Some(id::<IdentityId>("identity-maya-fixture")),
        "Maya Fixture",
        ParticipantRole::Member,
        -86_400_000,
    );
    let participant_alex_group = participant(
        "participant-alex-group",
        "person-alex-fixture",
        Some(identity.id.clone()),
        "Alex Fixture",
        ParticipantRole::Owner,
        -172_800_000,
    );
    let participant_maya_group = participant(
        "participant-maya-group",
        "person-maya-fixture",
        Some(id::<IdentityId>("identity-maya-fixture")),
        "Maya Fixture",
        ParticipantRole::Member,
        -172_800_000,
    );
    let participant_jordan_group = participant(
        "participant-jordan-group",
        "person-jordan-fixture",
        Some(id::<IdentityId>("identity-jordan-fixture")),
        "Jordan Fixture",
        ParticipantRole::Administrator,
        -172_800_000,
    );
    let participant_devon_group = participant(
        "participant-devon-group",
        "person-devon-fixture",
        Some(id::<IdentityId>("identity-devon-fixture")),
        "Devon Fixture",
        ParticipantRole::Member,
        -172_800_000,
    );

    let direct_conversation_id = id::<ConversationId>("conversation-direct-fixture");
    let group_conversation_id = id::<ConversationId>("conversation-group-fixture");
    let direct_conversation = Conversation {
        id: direct_conversation_id.clone(),
        kind: ConversationKind::Direct,
        title: None,
        participants: vec![
            participant_alex_direct.clone(),
            participant_maya_direct.clone(),
        ],
        created_at: Some(ts(-86_400_000)),
        updated_at: Some(ts(8_000)),
        extensions: vec![extension("fixture.conversation", "mode", "direct")],
    };
    let group_conversation = Conversation {
        id: group_conversation_id.clone(),
        kind: ConversationKind::Group(Box::new(GroupDetails {
            title: Some("Weekend Fixture Plans".to_owned()),
            owner: Some(participant_alex_group.id.clone()),
            avatar: Some(group_avatar.clone()),
        })),
        title: Some("Weekend Fixture Plans".to_owned()),
        participants: vec![
            participant_alex_group.clone(),
            participant_maya_group.clone(),
            participant_jordan_group.clone(),
            participant_devon_group.clone(),
        ],
        created_at: Some(ts(-172_800_000)),
        updated_at: Some(ts(9_000)),
        extensions: vec![extension("fixture.conversation", "mode", "group")],
    };

    let message_text_id = id::<MessageId>("message-text-fixture");
    let message_text = Message {
        id: message_text_id.clone(),
        conversation_id: direct_conversation_id.clone(),
        sender: Some(participant_maya_direct.id.clone()),
        sent_at: ts(1_000),
        parts: vec![MessagePart::Text(TextPart {
            text: "The garden is looking great today 😀.".to_owned(),
            formatting: vec![TextFormatting {
                range: TextRange::new(4, 10),
                style: TextStyle::Bold,
            }],
        })],
        mutations: vec![],
        reactions: vec![
            Reaction {
                id: id::<ReactionId>("reaction-love-fixture"),
                message_id: message_text_id.clone(),
                participant_id: participant_alex_direct.id.clone(),
                kind: ReactionKind::Love,
                created_at: ts(1_500),
                removed_at: None,
            },
            Reaction {
                id: id::<ReactionId>("reaction-removed-fixture"),
                message_id: message_text_id.clone(),
                participant_id: participant_maya_direct.id.clone(),
                kind: ReactionKind::Custom("sparkle".to_owned()),
                created_at: ts(1_600),
                removed_at: Some(ts(1_700)),
            },
        ],
        delivery: DeliveryState::Delivered,
        delivery_receipts: vec![DeliveryReceipt {
            participant_id: participant_alex_direct.id.clone(),
            state: DeliveryState::Delivered,
            at: Some(ts(2_000)),
        }],
        read_state: ReadState::Unread,
        read_receipts: vec![],
        reply_to: None,
        extensions: vec![extension("fixture.message", "source", "incoming")],
    };

    let message_reply_id = id::<MessageId>("message-reply-fixture");
    let message_reply = Message {
        id: message_reply_id,
        conversation_id: direct_conversation_id.clone(),
        sender: Some(participant_alex_direct.id.clone()),
        sent_at: ts(3_000),
        parts: vec![
            MessagePart::Text(TextPart {
                text: "I took a look—here is the latest.".to_owned(),
                formatting: vec![TextFormatting {
                    range: TextRange::new(0, 1),
                    style: TextStyle::Italic,
                }],
            }),
            MessagePart::Attachment(litebubbles_core::AttachmentPart {
                attachment: garden_photo.clone(),
                caption: Some(TextPart {
                    text: "The raised bed is ready.".to_owned(),
                    formatting: vec![],
                }),
            }),
            MessagePart::Attachment(litebubbles_core::AttachmentPart {
                attachment: voice_note.clone(),
                caption: None,
            }),
            MessagePart::LinkPreview(litebubbles_core::LinkPreview {
                url: "https://links.fixture.test/garden".to_owned(),
                title: Some("Fixture garden notes".to_owned()),
                summary: Some("Synthetic preview data for UI tests.".to_owned()),
                image: Some(garden_photo.clone()),
            }),
            MessagePart::Location(Location {
                point: GeoPoint {
                    latitude_e6: 12_345_678,
                    longitude_e6: -9_876_543,
                },
                accuracy_meters: Some(25),
                observed_at: Some(ts(2_900)),
            }),
            MessagePart::Contact(ContactCard {
                person: Some(id::<PersonId>("person-jordan-fixture")),
                display_name: "Jordan Fixture".to_owned(),
                identities: vec![IdentityAddress::Other {
                    kind: "fixture".to_owned(),
                    value: "jordan-contact".to_owned(),
                }],
            }),
            MessagePart::ServiceExtension(extension(
                "fixture.message",
                "composer-state",
                "multipart",
            )),
        ],
        mutations: vec![],
        reactions: vec![],
        delivery: DeliveryState::Sent,
        delivery_receipts: vec![DeliveryReceipt {
            participant_id: participant_maya_direct.id.clone(),
            state: DeliveryState::Sent,
            at: Some(ts(3_500)),
        }],
        read_state: ReadState::Read { at: ts(4_000) },
        read_receipts: vec![ReadReceipt {
            participant_id: participant_maya_direct.id.clone(),
            at: ts(4_000),
        }],
        reply_to: Some(message_text_id),
        extensions: vec![extension("fixture.message", "source", "reply")],
    };

    let edited_message_id = id::<MessageId>("message-edited-fixture");
    let edited_message = Message {
        id: edited_message_id.clone(),
        conversation_id: group_conversation_id.clone(),
        sender: Some(participant_jordan_group.id.clone()),
        sent_at: ts(5_000),
        parts: vec![MessagePart::Text(TextPart {
            text: "Let us meet at the north entrance.".to_owned(),
            formatting: vec![],
        })],
        mutations: vec![
            MessageMutation {
                id: id::<MessageMutationId>("mutation-edit-fixture"),
                message_id: edited_message_id.clone(),
                actor: Some(participant_jordan_group.id.clone()),
                occurred_at: ts(5_500),
                kind: MessageMutationKind::Edit {
                    parts: vec![MessagePart::Text(TextPart {
                        text: "Let us meet at the west entrance.".to_owned(),
                        formatting: vec![],
                    })],
                },
            },
            MessageMutation {
                id: id::<MessageMutationId>("mutation-service-fixture"),
                message_id: edited_message_id.clone(),
                actor: None,
                occurred_at: ts(5_600),
                kind: MessageMutationKind::ServiceExtension(extension(
                    "fixture.message",
                    "edit-source",
                    "scripted",
                )),
            },
        ],
        reactions: vec![Reaction {
            id: id::<ReactionId>("reaction-like-edited-fixture"),
            message_id: edited_message_id,
            participant_id: participant_maya_group.id.clone(),
            kind: ReactionKind::Like,
            created_at: ts(5_700),
            removed_at: None,
        }],
        delivery: DeliveryState::Delivered,
        delivery_receipts: vec![DeliveryReceipt {
            participant_id: participant_alex_group.id.clone(),
            state: DeliveryState::Delivered,
            at: Some(ts(6_000)),
        }],
        read_state: ReadState::Read { at: ts(6_500) },
        read_receipts: vec![ReadReceipt {
            participant_id: participant_alex_group.id.clone(),
            at: ts(6_500),
        }],
        reply_to: None,
        extensions: vec![],
    };

    let unsent_message_id = id::<MessageId>("message-unsent-fixture");
    let unsent_message = Message {
        id: unsent_message_id.clone(),
        conversation_id: group_conversation_id.clone(),
        sender: Some(participant_alex_group.id.clone()),
        sent_at: ts(7_000),
        parts: vec![MessagePart::Text(TextPart {
            text: "This fixture message was removed.".to_owned(),
            formatting: vec![],
        })],
        mutations: vec![MessageMutation {
            id: id::<MessageMutationId>("mutation-unsend-fixture"),
            message_id: unsent_message_id.clone(),
            actor: Some(participant_alex_group.id.clone()),
            occurred_at: ts(7_500),
            kind: MessageMutationKind::Unsend {
                reason: Some("fixture scenario".to_owned()),
            },
        }],
        reactions: vec![],
        delivery: DeliveryState::Delivered,
        delivery_receipts: vec![],
        read_state: ReadState::Unread,
        read_receipts: vec![],
        reply_to: None,
        extensions: vec![],
    };

    let queued_message = Message {
        id: id::<MessageId>("message-queued-fixture"),
        conversation_id: group_conversation_id.clone(),
        sender: Some(participant_alex_group.id.clone()),
        sent_at: ts(8_000),
        parts: vec![MessagePart::Text(TextPart {
            text: "This message is waiting to send.".to_owned(),
            formatting: vec![],
        })],
        mutations: vec![],
        reactions: vec![],
        delivery: DeliveryState::Queued,
        delivery_receipts: vec![],
        read_state: ReadState::Unread,
        read_receipts: vec![],
        reply_to: None,
        extensions: vec![],
    };

    let failed_message = Message {
        id: id::<MessageId>("message-failed-fixture"),
        conversation_id: group_conversation_id.clone(),
        sender: Some(participant_alex_group.id.clone()),
        sent_at: ts(8_500),
        parts: vec![MessagePart::Text(TextPart {
            text: "This message demonstrates a failed delivery.".to_owned(),
            formatting: vec![],
        })],
        mutations: vec![],
        reactions: vec![],
        delivery: DeliveryState::Failed {
            reason: Some("fixture delivery failure".to_owned()),
        },
        delivery_receipts: vec![],
        read_state: ReadState::Unread,
        read_receipts: vec![],
        reply_to: None,
        extensions: vec![],
    };

    let call = Call {
        id: id::<CallId>("call-video-fixture"),
        conversation_id: group_conversation_id.clone(),
        kind: CallKind::Video,
        state: CallState::Active,
        participants: vec![
            CallParticipant {
                participant_id: participant_alex_group.id.clone(),
                state: CallParticipantState::Connected,
            },
            CallParticipant {
                participant_id: participant_maya_group.id.clone(),
                state: CallParticipantState::Connected,
            },
            CallParticipant {
                participant_id: participant_jordan_group.id.clone(),
                state: CallParticipantState::Connecting,
            },
        ],
        started_at: Some(ts(10_000)),
        ended_at: None,
        extensions: vec![extension("fixture.call", "transport", "synthetic")],
    };

    let find_my_items = vec![
        FindMyItem {
            id: id::<FindMyItemId>("find-my-fixture-phone"),
            owner: account_id.clone(),
            name: "Fixture Phone".to_owned(),
            kind: FindMyItemKind::Device,
            status: FindMyItemStatus::Online,
            location: Some(Location {
                point: GeoPoint {
                    latitude_e6: 11_111_111,
                    longitude_e6: -22_222_222,
                },
                accuracy_meters: Some(40),
                observed_at: Some(ts(11_000)),
            }),
            battery_percent: Some(82),
            updated_at: Some(ts(11_000)),
            extensions: vec![],
        },
        FindMyItem {
            id: id::<FindMyItemId>("find-my-fixture-tag"),
            owner: account_id.clone(),
            name: "Fixture Tag".to_owned(),
            kind: FindMyItemKind::AirTag,
            status: FindMyItemStatus::Lost,
            location: None,
            battery_percent: Some(17),
            updated_at: Some(ts(12_000)),
            extensions: vec![extension("fixture.find-my", "lost-mode", "enabled")],
        },
    ];

    let shared_album = SharedAlbum {
        id: id::<SharedAlbumId>("album-fixture-weekend"),
        title: "Fixture Weekend Album".to_owned(),
        owner: id::<PersonId>("person-maya-fixture"),
        members: vec![
            SharedAlbumMember {
                person_id: id::<PersonId>("person-alex-fixture"),
                can_contribute: true,
            },
            SharedAlbumMember {
                person_id: id::<PersonId>("person-jordan-fixture"),
                can_contribute: false,
            },
        ],
        assets: vec![SharedAlbumAsset {
            id: id::<SharedAlbumAssetId>("album-asset-fixture-photo"),
            album_id: id::<SharedAlbumId>("album-fixture-weekend"),
            kind: AlbumAssetKind::Photo,
            attachment: album_photo.clone(),
            caption: Some(TextPart {
                text: "A synthetic shared-album caption.".to_owned(),
                formatting: vec![],
            }),
            contributed_by: Some(id::<PersonId>("person-maya-fixture")),
            created_at: Some(ts(13_000)),
            extensions: vec![],
        }],
        created_at: Some(ts(12_500)),
        extensions: vec![extension("fixture.album", "sharing", "synthetic")],
    };

    let fixture = FixtureSet {
        account,
        people,
        attachments: vec![
            avatar_maya,
            group_avatar,
            garden_photo,
            voice_note,
            map_file,
            album_photo,
        ],
        conversations: vec![direct_conversation, group_conversation],
        messages: vec![
            message_text,
            message_reply,
            edited_message,
            unsent_message,
            queued_message,
            failed_message,
        ],
        typing: vec![
            TypingIndicator {
                conversation_id: direct_conversation_id,
                participant_id: participant_maya_direct.id,
                state: TypingState::Started,
                observed_at: ts(14_000),
            },
            TypingIndicator {
                conversation_id: group_conversation_id,
                participant_id: participant_jordan_group.id,
                state: TypingState::Stopped,
                observed_at: ts(14_500),
            },
        ],
        calls: vec![call],
        find_my_items,
        shared_albums: vec![shared_album],
    };

    fixture
        .validate()
        .expect("representative mock fixture must be valid");
    fixture
}

/// Builds the standard deterministic live-event sequence for a fixture.
pub fn representative_events(fixture: &FixtureSet) -> EventScript {
    let direct_id = fixture.conversations[0].id.clone();
    let group_id = fixture.conversations[1].id.clone();
    let maya_direct_id = fixture.conversations[0].participants[1].id.clone();

    let typing = fixture.typing[0].clone();

    let mut read_message = fixture
        .message(&id::<MessageId>("message-text-fixture"))
        .expect("representative event message exists")
        .clone();
    read_message.read_state = ReadState::Read { at: ts(15_000) };
    read_message.read_receipts = vec![ReadReceipt {
        participant_id: maya_direct_id.clone(),
        at: ts(15_000),
    }];

    let mut delivered_message = fixture
        .message(&id::<MessageId>("message-queued-fixture"))
        .expect("representative queued message exists")
        .clone();
    delivered_message.delivery = DeliveryState::Delivered;
    delivered_message.delivery_receipts = vec![DeliveryReceipt {
        participant_id: fixture.conversations[1].participants[1].id.clone(),
        state: DeliveryState::Delivered,
        at: Some(ts(15_500)),
    }];

    let mut added_message = fixture
        .message(&id::<MessageId>("message-reply-fixture"))
        .expect("representative reply message exists")
        .clone();
    added_message.id = id::<MessageId>("message-live-added-fixture");
    added_message.sent_at = ts(16_000);
    added_message.reply_to = None;

    let mut ended_call = fixture.calls[0].clone();
    ended_call.state = CallState::Ended;
    ended_call.ended_at = Some(ts(16_500));

    let mut offline_item = fixture.find_my_items[0].clone();
    offline_item.status = FindMyItemStatus::Offline;
    offline_item.updated_at = Some(ts(17_000));

    let service_extension = ServiceExtension::new(
        "fixture.live",
        "banner",
        Some("text/plain".to_owned()),
        b"A scripted fixture event".to_vec(),
    )
    .expect("fixture service extension is valid");

    EventScript::new(vec![
        event(
            "event-001-typing",
            14_900,
            BackendEventKind::TypingChanged(typing),
        ),
        event(
            "event-002-read",
            15_000,
            BackendEventKind::MessageChanged(read_message),
        ),
        event(
            "event-003-delivery",
            15_500,
            BackendEventKind::MessageChanged(delivered_message),
        ),
        event(
            "event-004-message-added",
            16_000,
            BackendEventKind::MessageAdded(added_message),
        ),
        event(
            "event-005-call-ended",
            16_500,
            BackendEventKind::CallChanged(ended_call),
        ),
        event(
            "event-006-find-my-offline",
            17_000,
            BackendEventKind::FindMyItemChanged(offline_item),
        ),
        event(
            "event-007-extension",
            17_500,
            BackendEventKind::ServiceExtension(service_extension),
        ),
        event(
            "event-008-message-removed",
            18_000,
            BackendEventKind::MessageRemoved {
                conversation_id: group_id,
                message_id: id::<MessageId>("message-unsent-fixture"),
            },
        ),
        event(
            "event-009-conversation-refresh",
            18_500,
            BackendEventKind::ConversationChanged(
                fixture
                    .conversation(&direct_id)
                    .expect("representative conversation exists")
                    .clone(),
            ),
        ),
    ])
}

fn event(id_value: &str, offset: i64, kind: BackendEventKind) -> BackendEvent {
    BackendEvent {
        id: id::<BackendEventId>(id_value),
        occurred_at: ts(offset),
        kind,
        extensions: vec![],
    }
}

fn participant(
    id_value: &str,
    person_id: &str,
    identity_id: Option<IdentityId>,
    display_name: &str,
    role: ParticipantRole,
    joined_offset: i64,
) -> Participant {
    Participant {
        id: id::<ParticipantId>(id_value),
        person_id: Some(id::<PersonId>(person_id)),
        identity_id,
        display_name: Some(display_name.to_owned()),
        role,
        joined_at: Some(ts(joined_offset)),
        left_at: None,
        extensions: vec![],
    }
}

fn extension(service: &str, name: &str, payload: &str) -> ServiceExtension {
    ServiceExtension::new(
        service,
        name,
        Some("text/plain".to_owned()),
        payload.as_bytes().to_vec(),
    )
    .expect("fixture service extension is valid")
}

fn id<T>(value: &str) -> T
where
    T: TryFrom<String, Error = IdError>,
{
    T::try_from(value.to_owned()).expect("fixture identifier is valid")
}

fn ts(offset: i64) -> Timestamp {
    Timestamp::from_millis(FIXTURE_EPOCH_MS + offset)
}
