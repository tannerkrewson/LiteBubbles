//! Versioned transport-neutral contract between the application and daemon.

use litebubbles_core::{Conversation, ConversationId, Message};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProtocolHello {
    pub version: u16,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Request {
    ListConversations,
    ListMessages {
        conversation_id: ConversationId,
        limit: u32,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Response {
    Conversations(Vec<Conversation>),
    Messages(Vec<Message>),
}
