//! Deterministic, credential-free backend fixtures for UI and daemon tests.
//!
//! [`FixtureSet`] contains backend-independent domain values. [`MockBackend`]
//! exposes those values together with a finite [`EventScript`] so tests can
//! replay live changes without a clock, network connection, or account data.
//!
//! ```
//! let mut backend = litebubbles_mock_backend::MockBackend::new();
//! assert_eq!(backend.fixture().conversations.len(), 2);
//! assert!(backend.next_event().is_some());
//! ```

use litebubbles_core::{
    Account, Attachment, BackendEvent, Conversation, ConversationId, DomainError, FindMyItem,
    Message, MessageId, Person, SharedAlbum, TypingIndicator,
};
use serde::{Deserialize, Serialize};

pub mod fixtures;

pub use fixtures::{representative, representative_events};

/// A complete synthetic state snapshot used by the mock backend.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FixtureSet {
    /// Account and account-owned identities/devices.
    pub account: Account,
    /// People known to the account.
    pub people: Vec<Person>,
    /// Attachments available to the fixture backend by stable ID.
    pub attachments: Vec<Attachment>,
    /// Conversations in deterministic display order.
    pub conversations: Vec<Conversation>,
    /// Messages in deterministic history order.
    pub messages: Vec<Message>,
    /// Current typing indicators.
    pub typing: Vec<TypingIndicator>,
    /// Calls known to the backend.
    pub calls: Vec<litebubbles_core::Call>,
    /// Find My items known to the account.
    pub find_my_items: Vec<FindMyItem>,
    /// Shared albums available to the account.
    pub shared_albums: Vec<SharedAlbum>,
}

impl FixtureSet {
    /// Validates every core object owned by the fixture.
    pub fn validate(&self) -> Result<(), DomainError> {
        self.account.validate()?;
        for conversation in &self.conversations {
            conversation.validate()?;
        }
        for message in &self.messages {
            message.validate()?;
        }
        for call in &self.calls {
            call.validate()?;
        }
        for album in &self.shared_albums {
            album.validate()?;
        }

        for message in &self.messages {
            let Some(conversation) = self.conversation(&message.conversation_id) else {
                return Err(DomainError::MissingRelationship("message conversation"));
            };
            if let Some(sender) = &message.sender
                && conversation.participant(sender).is_none()
            {
                return Err(DomainError::InvalidRelationship("message sender"));
            }
            if let Some(reply_to) = &message.reply_to {
                let Some(parent) = self.message(reply_to) else {
                    return Err(DomainError::MissingRelationship("message reply"));
                };
                if parent.conversation_id != message.conversation_id {
                    return Err(DomainError::InvalidRelationship("message reply"));
                }
            }
            for reaction in &message.reactions {
                if conversation.participant(&reaction.participant_id).is_none() {
                    return Err(DomainError::InvalidRelationship("reaction participant"));
                }
            }
            for receipt in &message.delivery_receipts {
                if conversation.participant(&receipt.participant_id).is_none() {
                    return Err(DomainError::InvalidRelationship(
                        "delivery receipt participant",
                    ));
                }
            }
            for receipt in &message.read_receipts {
                if conversation.participant(&receipt.participant_id).is_none() {
                    return Err(DomainError::InvalidRelationship("read receipt participant"));
                }
            }
        }

        Ok(())
    }

    /// Finds a conversation by its stable ID.
    pub fn conversation(&self, id: &ConversationId) -> Option<&Conversation> {
        self.conversations
            .iter()
            .find(|conversation| &conversation.id == id)
    }

    /// Finds a message by its stable ID.
    pub fn message(&self, id: &MessageId) -> Option<&Message> {
        self.messages.iter().find(|message| &message.id == id)
    }

    /// Returns messages belonging to one conversation in fixture order.
    pub fn messages_for(&self, id: &ConversationId) -> impl Iterator<Item = &Message> {
        self.messages
            .iter()
            .filter(move |message| &message.conversation_id == id)
    }

    /// Finds an attachment by its stable ID.
    pub fn attachment(&self, id: &litebubbles_core::AttachmentId) -> Option<&Attachment> {
        self.attachments
            .iter()
            .find(|attachment| &attachment.id == id)
    }
}

/// An ordered, immutable set of events that can be replayed by a test.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct EventScript {
    events: Vec<BackendEvent>,
}

impl EventScript {
    /// Creates a script whose event order is exactly the supplied order.
    pub fn new(events: Vec<BackendEvent>) -> Self {
        Self { events }
    }

    /// Returns events in their deterministic replay order.
    pub fn events(&self) -> &[BackendEvent] {
        &self.events
    }

    /// Returns the number of events in this script.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Returns whether this script has no events.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Iterates over the script without consuming it.
    pub fn iter(&self) -> impl Iterator<Item = &BackendEvent> {
        self.events.iter()
    }
}

impl<'a> IntoIterator for &'a EventScript {
    type Item = &'a BackendEvent;
    type IntoIter = std::slice::Iter<'a, BackendEvent>;

    fn into_iter(self) -> Self::IntoIter {
        self.events.iter()
    }
}

/// A stateful test backend that replays a fixture snapshot and scripted events.
#[derive(Clone, Debug)]
pub struct MockBackend {
    fixture: FixtureSet,
    script: EventScript,
    next_event: usize,
}

impl Default for MockBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MockBackend {
    /// Creates the standard representative fixture and event script.
    pub fn new() -> Self {
        let fixture = representative();
        let script = representative_events(&fixture);
        Self::with_script(fixture, script)
    }

    /// Creates a backend with a caller-provided fixture and no live events.
    ///
    /// Use [`Self::with_script`] when a custom fixture needs a custom event
    /// sequence.
    pub fn with_fixture(fixture: FixtureSet) -> Self {
        Self::with_script(fixture, EventScript::default())
    }

    /// Creates a backend with fully caller-controlled state and event order.
    pub fn with_script(fixture: FixtureSet, script: EventScript) -> Self {
        Self {
            fixture,
            script,
            next_event: 0,
        }
    }

    /// Returns the immutable fixture snapshot.
    pub fn fixture(&self) -> &FixtureSet {
        &self.fixture
    }

    /// Returns the immutable event script.
    pub fn script(&self) -> &EventScript {
        &self.script
    }

    /// Returns the next scripted event, advancing the replay cursor.
    pub fn next_event(&mut self) -> Option<BackendEvent> {
        let event = self.script.events.get(self.next_event).cloned();
        if event.is_some() {
            self.next_event += 1;
        }
        event
    }

    /// Returns the number of scripted events that have not been replayed.
    pub fn remaining_events(&self) -> usize {
        self.script.len().saturating_sub(self.next_event)
    }

    /// Restarts scripted event replay at the first event.
    pub fn reset_events(&mut self) {
        self.next_event = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn representative_fixture_is_valid_and_lookupable() {
        let fixture = representative();

        assert!(fixture.validate().is_ok());
        assert_eq!(fixture.people.len(), 4);
        assert_eq!(fixture.conversations.len(), 2);
        assert!(fixture.messages_for(&fixture.conversations[0].id).count() >= 2);
        assert!(fixture.attachment(&fixture.attachments[0].id).is_some());
    }

    #[test]
    fn representative_fixture_round_trips_through_json() {
        let fixture = representative();
        let encoded = serde_json::to_string_pretty(&fixture).expect("fixture serializes");
        let decoded: FixtureSet = serde_json::from_str(&encoded).expect("fixture deserializes");

        assert_eq!(decoded, fixture);
    }

    #[test]
    fn representative_fixture_covers_requested_core_shapes() {
        use litebubbles_core::{
            BackendEventKind, CallState, ConversationKind, DeliveryState, MessageMutationKind,
            MessagePart, ReadState, TypingState,
        };

        let fixture = representative();
        let mut parts = fixture.messages.iter().flat_map(|message| &message.parts);

        assert!(
            parts
                .clone()
                .any(|part| matches!(part, MessagePart::Text(_)))
        );
        assert!(
            parts
                .clone()
                .any(|part| matches!(part, MessagePart::Attachment(_)))
        );
        assert!(
            parts
                .clone()
                .any(|part| matches!(part, MessagePart::LinkPreview(_)))
        );
        assert!(
            parts
                .clone()
                .any(|part| matches!(part, MessagePart::Location(_)))
        );
        assert!(
            parts
                .clone()
                .any(|part| matches!(part, MessagePart::Contact(_)))
        );
        assert!(parts.any(|part| matches!(part, MessagePart::ServiceExtension(_))));
        assert!(
            fixture
                .messages
                .iter()
                .any(|message| message.reply_to.is_some())
        );
        assert!(
            fixture
                .messages
                .iter()
                .any(|message| !message.reactions.is_empty())
        );
        assert!(fixture.messages.iter().any(|message| {
            message
                .mutations
                .iter()
                .any(|mutation| matches!(&mutation.kind, MessageMutationKind::Edit { .. }))
        }));
        assert!(fixture.messages.iter().any(|message| {
            message
                .mutations
                .iter()
                .any(|mutation| matches!(&mutation.kind, MessageMutationKind::Unsend { .. }))
        }));
        assert!(
            fixture
                .conversations
                .iter()
                .any(|conversation| matches!(&conversation.kind, ConversationKind::Group(_)))
        );
        assert!(
            fixture
                .messages
                .iter()
                .any(|message| matches!(&message.delivery, DeliveryState::Queued))
        );
        assert!(
            fixture
                .messages
                .iter()
                .any(|message| matches!(&message.delivery, DeliveryState::Failed { .. }))
        );
        assert!(
            fixture
                .messages
                .iter()
                .any(|message| matches!(&message.read_state, ReadState::Read { .. }))
        );
        assert!(
            fixture
                .messages
                .iter()
                .any(|message| matches!(&message.read_state, ReadState::Unread))
        );
        assert!(
            fixture
                .typing
                .iter()
                .any(|typing| typing.state == TypingState::Started)
        );
        assert!(
            fixture
                .typing
                .iter()
                .any(|typing| typing.state == TypingState::Stopped)
        );
        assert!(
            fixture
                .calls
                .iter()
                .any(|call| call.state == CallState::Active)
        );
        assert!(!fixture.find_my_items.is_empty());

        let script = representative_events(&fixture);
        assert!(script.events().iter().any(|event| matches!(
            &event.kind,
            BackendEventKind::CallChanged(_) | BackendEventKind::FindMyItemChanged(_)
        )));
    }

    #[test]
    fn event_script_replays_deterministically_and_can_reset() {
        let mut first = MockBackend::new();
        let mut second = MockBackend::new();
        let first_ids: Vec<_> =
            std::iter::from_fn(|| first.next_event().map(|event| event.id)).collect();
        let second_ids: Vec<_> =
            std::iter::from_fn(|| second.next_event().map(|event| event.id)).collect();

        assert!(!first_ids.is_empty());
        assert_eq!(first_ids, second_ids);
        assert_eq!(first.remaining_events(), 0);

        first.reset_events();
        assert_eq!(
            first.next_event().map(|event| event.id),
            first_ids.first().cloned()
        );
    }

    #[test]
    fn event_script_round_trips_through_json() {
        let fixture = representative();
        let script = representative_events(&fixture);
        let encoded = serde_json::to_string(&script).expect("script serializes");
        let decoded: EventScript = serde_json::from_str(&encoded).expect("script deserializes");

        assert_eq!(decoded, script);
    }
}
