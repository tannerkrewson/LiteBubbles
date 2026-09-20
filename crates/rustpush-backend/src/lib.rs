//! Adapter boundary for the pinned rustpush revision.
//!
//! Upstream values are converted here and are never exposed to the rest of the
//! workspace. In particular, core owns the resulting message semantics while
//! this crate owns rustpush configuration, error, and representation details.

use std::fmt;

use litebubbles_core::{
    Attachment, AttachmentContent, AttachmentId, AttachmentKind, DomainError, Message, MessageId,
    MessagePart, ParticipantId, ServiceExtension, TextFormatting, TextPart, TextRange, TextStyle,
    Timestamp,
};
use thiserror::Error;

pub const RUSTPUSH_REVISION: &str = "f35c4ee062b3c3eae54dc96b89b90ee99f5e1d0c";

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("invalid rustpush backend configuration: {0}")]
    InvalidConfiguration(&'static str),
    #[error("invalid core value: {0}")]
    Core(#[from] DomainError),
    #[error("invalid upstream identifier: {0}")]
    InvalidIdentifier(#[from] litebubbles_core::IdError),
    #[error("unsupported rustpush message variant: {0}")]
    UnsupportedMessage(&'static str),
    #[error("rustpush operation failed ({kind}): {message}")]
    Upstream {
        kind: UpstreamErrorKind,
        message: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackendMode {
    Relay { host: String, code: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendConfig {
    mode: BackendMode,
}

impl BackendConfig {
    pub fn relay(host: impl Into<String>, code: impl Into<String>) -> Result<Self, BackendError> {
        let host = host.into();
        let code = code.into();
        if host.trim().is_empty() {
            return Err(BackendError::InvalidConfiguration(
                "relay host must not be empty",
            ));
        }
        if code.trim().is_empty() {
            return Err(BackendError::InvalidConfiguration(
                "relay code must not be empty",
            ));
        }
        Ok(Self {
            mode: BackendMode::Relay { host, code },
        })
    }

    pub fn from_relay_config(config: &rustpush::RelayConfig) -> Result<Self, BackendError> {
        Self::relay(config.host.clone(), config.code.clone())
    }

    pub fn mode(&self) -> &BackendMode {
        &self.mode
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpstreamErrorKind {
    MalformedMessage,
    NoValidTargets,
    Disconnected,
    TimedOut,
    Relay,
    Other,
}

impl fmt::Display for UpstreamErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MalformedMessage => "malformed message",
            Self::NoValidTargets => "no valid targets",
            Self::Disconnected => "disconnected",
            Self::TimedOut => "timed out",
            Self::Relay => "relay",
            Self::Other => "other",
        })
    }
}

impl From<rustpush::PushError> for BackendError {
    fn from(error: rustpush::PushError) -> Self {
        let kind = match &error {
            rustpush::PushError::BadMsg => UpstreamErrorKind::MalformedMessage,
            rustpush::PushError::NoValidTargets => UpstreamErrorKind::NoValidTargets,
            rustpush::PushError::NotConnected | rustpush::PushError::ConnectionClosed(_) => {
                UpstreamErrorKind::Disconnected
            }
            rustpush::PushError::SendTimedOut => UpstreamErrorKind::TimedOut,
            rustpush::PushError::RelayError(_, _) => UpstreamErrorKind::Relay,
            _ => UpstreamErrorKind::Other,
        };
        Self::Upstream {
            kind,
            message: error.to_string(),
        }
    }
}

#[derive(Debug)]
pub struct Backend {
    config: BackendConfig,
}

impl Backend {
    pub fn new(config: BackendConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &BackendConfig {
        &self.config
    }

    /// Converts one representative rustpush message into a core message.
    /// Unsupported upstream variants fail explicitly instead of leaking an
    /// upstream enum or silently dropping data.
    pub fn translate_message(
        &self,
        upstream: &rustpush::MessageInst,
    ) -> Result<Message, BackendError> {
        let conversation =
            upstream
                .conversation
                .as_ref()
                .ok_or(BackendError::InvalidConfiguration(
                    "message is missing conversation data",
                ))?;
        let conversation_id = conversation_id(conversation)?;
        let sender = upstream
            .sender
            .as_deref()
            .map(ParticipantId::new)
            .transpose()?;
        let parts = translate_message_parts(&upstream.message)?;
        let message = Message {
            id: MessageId::new(upstream.id.clone())?,
            conversation_id,
            sender,
            sent_at: Timestamp::from_millis(upstream.sent_timestamp as i64),
            parts,
            mutations: Vec::new(),
            reactions: Vec::new(),
            delivery: litebubbles_core::DeliveryState::Queued,
            delivery_receipts: Vec::new(),
            read_state: litebubbles_core::ReadState::Unread,
            read_receipts: Vec::new(),
            reply_to: None,
            extensions: vec![ServiceExtension::new(
                "rustpush",
                "message-service",
                None,
                match &upstream.message {
                    rustpush::Message::Message(normal) => {
                        message_service(&normal.service).as_bytes().to_vec()
                    }
                    _ => Vec::new(),
                },
            )?],
        };
        message.validate()?;
        Ok(message)
    }
}

fn conversation_id(
    conversation: &rustpush::ConversationData,
) -> Result<litebubbles_core::ConversationId, BackendError> {
    let value = conversation
        .sender_guid
        .clone()
        .or_else(|| {
            (!conversation.participants.is_empty()).then(|| conversation.participants.join(","))
        })
        .ok_or(BackendError::InvalidConfiguration(
            "message conversation has no stable identifier",
        ))?;
    Ok(litebubbles_core::ConversationId::new(value)?)
}

fn translate_message_parts(message: &rustpush::Message) -> Result<Vec<MessagePart>, BackendError> {
    let rustpush::Message::Message(normal) = message else {
        return Err(BackendError::UnsupportedMessage(message_variant(message)));
    };
    normal
        .parts
        .0
        .iter()
        .map(|part| match &part.part {
            rustpush::MessagePart::Text(text, format) => {
                let formatting = translate_text_format(text, format)?;
                Ok(MessagePart::Text(TextPart {
                    text: text.clone(),
                    formatting,
                }))
            }
            rustpush::MessagePart::Mention(handle, text) => {
                let participant_id = ParticipantId::new(handle.clone())?;
                let end = text.encode_utf16().count() as u32;
                Ok(MessagePart::Text(TextPart {
                    text: text.clone(),
                    formatting: vec![TextFormatting {
                        range: TextRange::new(0, end),
                        style: TextStyle::Mention { participant_id },
                    }],
                }))
            }
            rustpush::MessagePart::Attachment(attachment) => {
                Ok(MessagePart::Attachment(translate_attachment(attachment)?))
            }
            rustpush::MessagePart::Object(object) => {
                Ok(MessagePart::ServiceExtension(ServiceExtension::new(
                    "rustpush",
                    "message-object",
                    None,
                    object.as_bytes().to_vec(),
                )?))
            }
        })
        .collect()
}

fn translate_text_format(
    text: &str,
    format: &rustpush::TextFormat,
) -> Result<Vec<TextFormatting>, BackendError> {
    let end = text.encode_utf16().count() as u32;
    let range = TextRange::new(0, end);
    Ok(match format {
        rustpush::TextFormat::Flags(flags) => [
            (flags.bold, TextStyle::Bold),
            (flags.italic, TextStyle::Italic),
            (flags.underline, TextStyle::Underline),
            (flags.strikethrough, TextStyle::Strikethrough),
        ]
        .into_iter()
        .filter_map(|(enabled, style)| enabled.then_some(TextFormatting { range, style }))
        .collect(),
        rustpush::TextFormat::Effect(effect) => vec![TextFormatting {
            range,
            style: TextStyle::Custom(ServiceExtension::new(
                "rustpush",
                "text-effect",
                Some("application/x-rustpush-text-effect".to_owned()),
                (*effect as u32).to_string().into_bytes(),
            )?),
        }],
    })
}

fn translate_attachment(
    attachment: &rustpush::Attachment,
) -> Result<litebubbles_core::AttachmentPart, BackendError> {
    let kind = match attachment.mime.split('/').next() {
        Some("image") => AttachmentKind::Image,
        Some("video") => AttachmentKind::Video,
        Some("audio") => AttachmentKind::Audio,
        _ => AttachmentKind::File,
    };
    let (content, byte_size) = match &attachment.a_type {
        rustpush::AttachmentType::Inline(data) => (
            AttachmentContent::Inline(data.clone()),
            Some(data.len() as u64),
        ),
        rustpush::AttachmentType::MMCS(file) => (
            AttachmentContent::External {
                token: format!("rustpush:mmcs:{}", file.object),
            },
            Some(file.size as u64),
        ),
    };
    Ok(litebubbles_core::AttachmentPart {
        attachment: Attachment {
            id: AttachmentId::new(format!("rustpush:{}:{}", attachment.name, attachment.part))?,
            kind,
            file_name: (!attachment.name.is_empty()).then(|| attachment.name.clone()),
            mime_type: (!attachment.mime.is_empty()).then(|| attachment.mime.clone()),
            byte_size,
            content,
            extensions: vec![ServiceExtension::new(
                "rustpush",
                "attachment-uti",
                Some("text/plain".to_owned()),
                attachment.uti_type.as_bytes().to_vec(),
            )?],
        },
        caption: None,
    })
}

fn message_variant(message: &rustpush::Message) -> &'static str {
    match message {
        rustpush::Message::Message(_) => "message",
        rustpush::Message::RenameMessage(_) => "rename",
        rustpush::Message::ChangeParticipants(_) => "change-participants",
        rustpush::Message::React(_) => "reaction",
        rustpush::Message::Delivered => "delivered",
        rustpush::Message::Read => "read",
        rustpush::Message::Typing(_, _) => "typing",
        rustpush::Message::Unsend(_) => "unsend",
        rustpush::Message::Edit(_) => "edit",
        rustpush::Message::IconChange(_) => "icon-change",
        rustpush::Message::EnableSmsActivation(_) => "enable-sms-activation",
        rustpush::Message::MessageReadOnDevice => "message-read-on-device",
        rustpush::Message::SmsConfirmSent(_) => "sms-confirm-sent",
        rustpush::Message::MarkUnread => "mark-unread",
        rustpush::Message::PeerCacheInvalidate => "peer-cache-invalidate",
        rustpush::Message::UpdateExtension(_) => "update-extension",
        rustpush::Message::Error(_) => "error",
        rustpush::Message::MoveToRecycleBin(_) => "move-to-recycle-bin",
        rustpush::Message::RecoverChat(_) => "recover-chat",
        rustpush::Message::PermanentDelete(_) => "permanent-delete",
        rustpush::Message::Unschedule => "unschedule",
        rustpush::Message::UpdateProfile(_) => "update-profile",
        rustpush::Message::UpdateProfileSharing(_) => "update-profile-sharing",
        rustpush::Message::ShareProfile(_) => "share-profile",
        rustpush::Message::NotifyAnyways => "notify-anyways",
        rustpush::Message::SetTranscriptBackground(_) => "set-transcript-background",
        rustpush::Message::RecoverProfile => "recover-profile",
    }
}

fn message_service(service: &rustpush::MessageType) -> &'static str {
    match service {
        rustpush::MessageType::IMessage => "imessage",
        rustpush::MessageType::SMS { .. } => "sms",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relay_config() -> rustpush::RelayConfig {
        serde_json::from_value(serde_json::json!({
            "version": {
                "software_build_id": "22G513",
                "software_name": "macOS",
                "software_version": "13.6.4",
                "serial_number": "SERIAL",
                "hardware_version": "iMac13,1",
                "unique_device_id": "UDID"
            },
            "icloud_ua": "Mozilla/5.0",
            "aoskit_version": "1",
            "dev_uuid": "device",
            "protocol_version": 1,
            "host": "https://relay.example",
            "code": "secret-code",
            "beeper_token": null,
            "udid": "udid"
        }))
        .expect("fixture must deserialize")
    }

    #[test]
    fn projects_upstream_relay_configuration() {
        let config = BackendConfig::from_relay_config(&relay_config()).expect("valid config");
        assert_eq!(
            config.mode(),
            &BackendMode::Relay {
                host: "https://relay.example".to_owned(),
                code: "secret-code".to_owned(),
            }
        );
    }

    #[test]
    fn translates_text_and_formatting_without_upstream_types_in_core() {
        let backend = Backend::new(BackendConfig::from_relay_config(&relay_config()).unwrap());
        let normal = rustpush::NormalMessage {
            parts: rustpush::MessageParts(vec![rustpush::IndexedMessagePart {
                part: rustpush::MessagePart::Text(
                    "hello".to_owned(),
                    rustpush::TextFormat::Flags(rustpush::TextFlags {
                        bold: true,
                        italic: false,
                        underline: false,
                        strikethrough: false,
                    }),
                ),
                idx: None,
                ext: None,
            }]),
            effect: None,
            reply_guid: None,
            reply_part: None,
            service: rustpush::MessageType::IMessage,
            subject: None,
            app: None,
            link_meta: None,
            voice: false,
            scheduled: None,
            embedded_profile: None,
        };
        let upstream = rustpush::MessageInst {
            id: "message-1".to_owned(),
            sender: Some("person-1".to_owned()),
            conversation: Some(rustpush::ConversationData {
                participants: vec!["person-1".to_owned(), "person-2".to_owned()],
                cv_name: None,
                sender_guid: Some("conversation-1".to_owned()),
                after_guid: None,
            }),
            message: rustpush::Message::Message(normal),
            sent_timestamp: 42,
            target: None,
            send_delivered: false,
            verification_failed: false,
            certified_context: None,
        };

        let message = backend
            .translate_message(&upstream)
            .expect("message translates");
        assert_eq!(message.id.as_str(), "message-1");
        assert_eq!(message.conversation_id.as_str(), "conversation-1");
        assert!(matches!(message.parts[0], MessagePart::Text(_)));
        assert_eq!(message.sent_at, Timestamp::from_millis(42));
    }

    #[test]
    fn converts_representative_upstream_errors() {
        let error = BackendError::from(rustpush::PushError::BadMsg);
        assert!(matches!(
            error,
            BackendError::Upstream {
                kind: UpstreamErrorKind::MalformedMessage,
                ..
            }
        ));
    }
}
