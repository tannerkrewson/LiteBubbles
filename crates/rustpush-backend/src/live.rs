//! The small production connection path used by the daemon setup commands.
//!
//! All rustpush types stay in this crate.  The daemon receives only an opaque
//! encrypted-at-rest session snapshot and high-level setup/send operations.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use keystore::software::{SoftwareEncryptor, SoftwareKeystore, SoftwareKeystoreState};
use litebubbles_validation_provider::ValidationError;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{MacHardwareInput, ValidationBackedMacOsConfig};

/// The services registered by the upstream OpenBubbles application for a
/// normal Messages-capable device.
pub static IDS_SERVICES: &[&rustpush::IDSService] = &[
    &rustpush::MADRID_SERVICE,
    &rustpush::findmy::MULTIPLEX_SERVICE,
    &rustpush::facetime::FACETIME_SERVICE,
    &rustpush::facetime::VIDEO_SERVICE,
];

#[derive(Debug, Error)]
pub enum LiveError {
    #[error("production validation setup is unavailable: {0}")]
    Validation(#[from] ValidationError),
    #[error("hardware activation input is invalid: {0}")]
    Hardware(String),
    #[error("the local rustpush keystore is unavailable")]
    Keystore,
    #[error("the saved LiteBubbles Apple session is invalid")]
    InvalidSession,
    #[error("Apple account credentials are required for first-time setup")]
    PasswordRequired,
    #[error("Apple account login requires an additional web step: {0}")]
    AdditionalLoginStep(String),
    #[error("Apple account authentication failed")]
    AppleAuthentication,
    #[error("Apple service request failed")]
    AppleService,
    #[error("Apple returned no usable messaging identity")]
    MissingIdentity,
    #[error("the active Apple identity has no registered messaging handle")]
    MissingHandle,
    #[error("no message arrived before the listen timeout")]
    ReceiveTimeout,
    #[error("local session serialization failed")]
    Serialization,
    #[error("local session storage failed")]
    LocalIo,
}

/// The only user interaction needed after the account password is submitted.
/// The code itself is never retained by the backend after the callback returns.
#[derive(Clone, Debug)]
pub enum TwoFactorPrompt {
    TrustedDevice,
    Sms { last_two_digits: String },
}

/// Opaque bytes suitable for Secret Service storage.  It contains rustpush
/// state, including key aliases and account tokens, so callers must not put it
/// in SQLite, D-Bus messages, logs, or command-line arguments.
pub struct SessionSnapshot {
    bytes: Vec<u8>,
}

impl SessionSnapshot {
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, LiveError> {
        let _: StoredSession = plist::from_bytes(&bytes).map_err(|_| LiveError::InvalidSession)?;
        Ok(Self { bytes })
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Serialize, Deserialize)]
struct StoredSession {
    account: Vec<u8>,
    aps: rustpush::APSState,
    users: Vec<rustpush::IDSUser>,
    identity: Vec<u8>,
}

/// A connected, registered rustpush session.  It intentionally exposes only
/// high-level operations and keeps the protocol objects private to this crate.
pub struct LiveSession {
    client: rustpush::IMClient,
    account: Vec<u8>,
}

impl LiveSession {
    /// Establish APS, log in when necessary, register IDS, and create the
    /// rustpush Messages client.  A missing snapshot means first-time setup.
    #[allow(clippy::too_many_arguments)]
    pub async fn connect(
        hardware: MacHardwareInput,
        snapshot: Option<&[u8]>,
        apple_id: &str,
        password: Option<&str>,
        anisette_path: PathBuf,
        cache_path: PathBuf,
        prompt: &mut dyn FnMut(TwoFactorPrompt) -> String,
    ) -> Result<Self, LiveError> {
        let stored = snapshot
            .map(|bytes| {
                plist::from_bytes::<StoredSession>(bytes).map_err(|_| LiveError::InvalidSession)
            })
            .transpose()?;
        let persisted = stored
            .as_ref()
            .map(|session| plist::from_bytes(&session.account))
            .transpose()
            .map_err(|_| LiveError::InvalidSession)?;
        let old_state = stored.as_ref().map(|session| session.aps.clone());

        let config = Arc::new(ValidationBackedMacOsConfig::from_default_provider(
            hardware,
        )?);
        let (connection, connection_error) =
            rustpush::APSConnectionResource::new(config.clone(), old_state).await;
        if connection_error.is_some() {
            return Err(LiveError::AppleService);
        }

        let client_info = rustpush::OSConfig::get_gsa_config(
            config.as_ref(),
            &*connection.state.read().await,
            false,
        );
        let anisette = rustpush::default_provider(client_info.clone(), anisette_path);
        let mut account = rustpush::AppleAccount::new_with_anisette(
            client_info,
            anisette,
            persisted,
            Box::new(|_| {}),
        )
        .map_err(|_| LiveError::AppleAuthentication)?;

        let mut users = stored
            .as_ref()
            .map(|session| session.users.clone())
            .unwrap_or_default();
        let mut identity = stored
            .as_ref()
            .filter(|session| !session.identity.is_empty())
            .map(|session| {
                rustpush::IDSNGMIdentity::restore(&session.identity, "litebubbles")
                    .map_err(|_| LiveError::InvalidSession)
            })
            .transpose()?;

        if account.get_pet().is_none() {
            let password = password.ok_or(LiveError::PasswordRequired)?;
            let mut hashed_password = Sha256::new();
            hashed_password.update(password.as_bytes());
            let hashed_password = hashed_password.finalize();
            let mut login_state = account
                .login_email_pass(apple_id, &hashed_password)
                .await
                .map_err(|_| LiveError::AppleAuthentication)?;

            loop {
                login_state = match login_state {
                    rustpush::LoginState::LoggedIn => break,
                    rustpush::LoginState::NeedsDevice2FA => account
                        .send_2fa_to_devices()
                        .await
                        .map_err(|_| LiveError::AppleAuthentication)?,
                    rustpush::LoginState::Needs2FAVerification => {
                        let code = prompt(TwoFactorPrompt::TrustedDevice);
                        account
                            .verify_2fa(code)
                            .await
                            .map_err(|_| LiveError::AppleAuthentication)?
                    }
                    rustpush::LoginState::NeedsSMS2FA => {
                        let extras = account
                            .get_auth_extras()
                            .await
                            .map_err(|_| LiveError::AppleAuthentication)?;
                        let phone = extras
                            .trusted_phone_numbers
                            .first()
                            .ok_or(LiveError::AppleAuthentication)?;
                        account
                            .send_sms_2fa_to_devices(phone.id)
                            .await
                            .map_err(|_| LiveError::AppleAuthentication)?
                    }
                    rustpush::LoginState::NeedsSMS2FAVerification(body) => {
                        let last_two_digits = account
                            .get_auth_extras()
                            .await
                            .ok()
                            .and_then(|extras| {
                                extras
                                    .trusted_phone_numbers
                                    .first()
                                    .map(|phone| phone.last_two_digits.clone())
                            })
                            .unwrap_or_else(|| "unknown".to_owned());
                        let code = prompt(TwoFactorPrompt::Sms { last_two_digits });
                        account
                            .verify_sms_2fa(code, body)
                            .await
                            .map_err(|_| LiveError::AppleAuthentication)?
                    }
                    rustpush::LoginState::NeedsExtraStep(step) => {
                        return Err(LiveError::AdditionalLoginStep(step));
                    }
                    rustpush::LoginState::NeedsLogin => account
                        .login_email_pass(apple_id, &hashed_password)
                        .await
                        .map_err(|_| LiveError::AppleAuthentication)?,
                };
            }
        }

        if account
            .persisted
            .as_ref()
            .is_some_and(|persisted| persisted.postdata_done != Some(true))
        {
            account
                .update_postdata("Apple Device", None, &["icloud", "imessage", "facetime"])
                .await
                .map_err(|_| LiveError::AppleAuthentication)?;
        }

        if users.is_empty() || identity.is_none() {
            let delegates = rustpush::login_apple_delegates(
                &account,
                None,
                config.as_ref(),
                &[
                    rustpush::LoginDelegate::IDS,
                    rustpush::LoginDelegate::MobileMe,
                ],
            )
            .await
            .map_err(|_| LiveError::AppleAuthentication)?;
            let ids = delegates.ids.ok_or(LiveError::MissingIdentity)?;
            let user = rustpush::authenticate_apple(ids, config.as_ref())
                .await
                .map_err(|_| LiveError::AppleAuthentication)?;
            let new_identity =
                rustpush::IDSNGMIdentity::new().map_err(|_| LiveError::AppleAuthentication)?;
            users = vec![user];
            rustpush::register(
                config.as_ref(),
                &*connection.state.read().await,
                IDS_SERVICES,
                &mut users,
                &new_identity,
            )
            .await
            .map_err(|_| LiveError::AppleAuthentication)?;
            identity = Some(new_identity);
        }

        let account = account
            .persisted
            .as_ref()
            .ok_or(LiveError::AppleAuthentication)
            .and_then(encode_plist)?;
        let identity = identity.ok_or(LiveError::MissingIdentity)?;
        let client = rustpush::IMClient::new(
            connection,
            users,
            identity,
            IDS_SERVICES,
            cache_path,
            config,
            Box::new(|_| {}),
        )
        .await;

        Ok(Self { client, account })
    }

    pub async fn send_text(&self, destination: &str, text: &str) -> Result<(), LiveError> {
        if destination.trim().is_empty() || text.is_empty() {
            return Err(LiveError::MissingHandle);
        }
        let sender = self
            .client
            .identity
            .get_handles()
            .await
            .into_iter()
            .next()
            .ok_or(LiveError::MissingHandle)?;
        let conversation = rustpush::ConversationData {
            participants: vec![sender.clone(), destination.trim().to_owned()],
            cv_name: None,
            sender_guid: None,
            after_guid: None,
        };
        let mut message = rustpush::MessageInst::new(
            conversation,
            &sender,
            rustpush::Message::Message(rustpush::NormalMessage::new(
                text.to_owned(),
                rustpush::MessageType::IMessage,
            )),
        );
        self.client
            .send(&mut message)
            .await
            .map_err(|_| LiveError::AppleService)?;
        Ok(())
    }

    pub async fn wait_for_message(&self, timeout: Duration) -> Result<(), LiveError> {
        let mut messages = self.client.conn.messages_cont.subscribe();
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(LiveError::ReceiveTimeout);
            }
            let message = tokio::time::timeout(remaining, messages.recv())
                .await
                .map_err(|_| LiveError::ReceiveTimeout)?
                .map_err(|_| LiveError::AppleService)?;
            if self
                .client
                .handle(message)
                .await
                .map_err(|_| LiveError::AppleService)?
                .is_some()
            {
                return Ok(());
            }
        }
    }

    pub async fn snapshot(&self) -> Result<SessionSnapshot, LiveError> {
        let users = self.client.identity.resource.users.read().await.clone();
        let identity = self
            .client
            .identity
            .resource
            .identity
            .save("litebubbles")
            .map_err(|_| LiveError::Serialization)?;
        let aps = self.client.conn.state.read().await.clone();
        let stored = StoredSession {
            account: self.account.clone(),
            aps,
            users,
            identity,
        };
        let bytes = encode_plist(&stored)?;
        Ok(SessionSnapshot { bytes })
    }
}

/// Initialize rustpush's software keystore with a key held by the caller in
/// Secret Service.  The file contains encrypted rustpush keys and is never
/// useful without that separate secret.
pub fn initialize_local_keystore(
    state_path: impl AsRef<Path>,
    encryption_key: [u8; 32],
) -> Result<(), LiveError> {
    let state_path = state_path.as_ref();
    let parent = state_path.parent().ok_or(LiveError::Keystore)?;
    fs::create_dir_all(parent).map_err(|_| LiveError::Keystore)?;
    set_private_directory_mode(parent)?;
    let state = match fs::read(state_path) {
        Ok(bytes) => plist::from_bytes(&bytes).map_err(|_| LiveError::Keystore)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            SoftwareKeystoreState::default()
        }
        Err(_) => return Err(LiveError::Keystore),
    };
    let path = state_path.to_owned();
    keystore::init_keystore(SoftwareKeystore {
        state: RwLock::new(state),
        update_state: Box::new(move |state| {
            let Ok(bytes) = encode_plist(state) else {
                return;
            };
            let Ok(mut file) = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&path)
            else {
                return;
            };
            let _ = file.write_all(&bytes).and_then(|_| file.sync_all());
            let _ = set_private_file_mode(&path);
        }),
        encryptor: SoftwareEncryptor(encryption_key),
    });
    Ok(())
}

fn encode_plist<T: Serialize>(value: &T) -> Result<Vec<u8>, LiveError> {
    let mut bytes = Vec::new();
    plist::to_writer_binary(&mut bytes, value).map_err(|_| LiveError::Serialization)?;
    Ok(bytes)
}

pub fn new_keystore_key() -> [u8; 32] {
    let mut key = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    key
}

fn set_private_directory_mode(path: &Path) -> Result<(), LiveError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| LiveError::Keystore)?;
    }
    Ok(())
}

fn set_private_file_mode(path: &Path) -> Result<(), LiveError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|_| LiveError::Keystore)?;
    }
    Ok(())
}
