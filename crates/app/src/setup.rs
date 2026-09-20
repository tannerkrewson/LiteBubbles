//! First-run setup and account UI state.
//!
//! [`SetupState`] is a backend-independent reducer. [`SetupController`] owns a
//! reducer and notifies the GTK view after each event, while [`SetupView`]
//! provides the reusable libadwaita/GTK4 pages for the flow.
//!
//! Integration boundary: the view emits [`SetupEvent`] values and consumes
//! [`BackendUpdate`] values supplied by a later daemon adapter. This module
//! does not own persistence, secret storage, network access, or account
//! discovery. It deliberately has no legacy-data import, detection,
//! migration, backup, coexistence, or reuse path.

use std::{cell::RefCell, collections::HashSet, fmt, rc::Rc};

use adw::prelude::*;

/// The page currently shown by the setup flow.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetupStage {
    /// The application has not started setup yet.
    FirstRun,
    /// The user can enter activation and provisioning inputs.
    Activation,
    /// Activation inputs have been submitted and are being provisioned.
    Provisioning,
    /// The user can enter the account login.
    AppleLogin,
    /// The backend requires a second factor.
    TwoFactor,
    /// The backend returned identities that need an explicit user choice.
    IdentitySelection,
    /// A backend operation is in flight or the selected identity is connecting.
    Connecting,
    /// Setup completed and the account connection is usable.
    Complete,
    /// A safe, retryable failure is being shown.
    Error,
}

impl SetupStage {
    /// Returns the stable GTK stack page name for this stage.
    pub const fn page_name(self) -> &'static str {
        match self {
            Self::FirstRun => "first-run",
            Self::Activation => "activation",
            Self::Provisioning => "provisioning",
            Self::AppleLogin => "apple-login",
            Self::TwoFactor => "two-factor",
            Self::IdentitySelection => "identity-selection",
            Self::Connecting => "connecting",
            Self::Complete => "complete",
            Self::Error => "error",
        }
    }
}

/// The connection status represented by the setup state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionState {
    /// No backend connection is currently being attempted.
    Disconnected,
    /// A backend operation or connection attempt is in flight.
    Connecting,
    /// The selected account identity is connected.
    Connected,
    /// The last backend operation failed.
    Failed,
}

impl ConnectionState {
    /// Returns a short, safe status label for the view.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Disconnected => "Not connected",
            Self::Connecting => "Connecting…",
            Self::Connected => "Connected",
            Self::Failed => "Connection needs attention",
        }
    }
}

/// A safe, user-presentable error that can be retried without exposing
/// backend details or secret input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoverableError {
    /// Required activation or provisioning fields were missing.
    InvalidActivationInput,
    /// Required login fields were missing.
    InvalidAppleLoginInput,
    /// The second-factor code was malformed.
    InvalidTwoFactorInput,
    /// Provisioning could not be completed.
    ProvisioningFailed,
    /// The account login was rejected.
    AuthenticationFailed,
    /// The second-factor code was rejected.
    TwoFactorRejected,
    /// The backend returned no usable identities.
    NoIdentityAvailable,
    /// The backend returned an unusable or duplicate identity list.
    InvalidIdentityList,
    /// The selected identity could not be applied.
    IdentitySelectionFailed,
    /// The connection was lost after setup or during an operation.
    ConnectionLost,
    /// The backend is unavailable or returned an unspecified safe failure.
    BackendUnavailable,
}

impl RecoverableError {
    /// Returns a safe title suitable for an error page.
    pub const fn title(self) -> &'static str {
        match self {
            Self::InvalidActivationInput => "Check the setup details",
            Self::InvalidAppleLoginInput => "Check the login details",
            Self::InvalidTwoFactorInput => "Enter a valid verification code",
            Self::ProvisioningFailed => "Setup could not be completed",
            Self::AuthenticationFailed => "The login was not accepted",
            Self::TwoFactorRejected => "The verification code was not accepted",
            Self::NoIdentityAvailable => "No identity is available",
            Self::InvalidIdentityList => "The identity list is unavailable",
            Self::IdentitySelectionFailed => "That identity could not be selected",
            Self::ConnectionLost => "The connection was lost",
            Self::BackendUnavailable => "The setup service is unavailable",
        }
    }

    /// Returns a safe description suitable for an error page.
    pub const fn description(self) -> &'static str {
        match self {
            Self::InvalidActivationInput => "Enter all required setup fields and try again.",
            Self::InvalidAppleLoginInput => "Enter the account and password, then try again.",
            Self::InvalidTwoFactorInput => "Use the numeric verification code from your device.",
            Self::ProvisioningFailed => {
                "The backend could not provision this device. You can retry the setup."
            }
            Self::AuthenticationFailed => {
                "The backend rejected the login. Check the details and try again."
            }
            Self::TwoFactorRejected => {
                "The code was not accepted. Request a new code or try again."
            }
            Self::NoIdentityAvailable => {
                "The backend did not return an identity that can be used here."
            }
            Self::InvalidIdentityList => {
                "The backend returned an invalid identity list. You can retry."
            }
            Self::IdentitySelectionFailed => {
                "The selected identity could not be activated. Choose it again or retry."
            }
            Self::ConnectionLost => "Check the connection and retry setup.",
            Self::BackendUnavailable => {
                "The setup service is unavailable right now. You can retry."
            }
        }
    }
}

/// An identity offered by the backend for explicit user selection.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct IdentityOption {
    id: String,
    label: String,
    address: Option<String>,
}

impl IdentityOption {
    /// Creates an identity option from backend-provided display data.
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        address: Option<String>,
    ) -> Result<Self, IdentityOptionError> {
        let id = id.into();
        let label = label.into();
        if id.trim().is_empty() {
            return Err(IdentityOptionError::EmptyId);
        }
        if label.trim().is_empty() {
            return Err(IdentityOptionError::EmptyLabel);
        }
        let address = address.filter(|value| !value.trim().is_empty());
        Ok(Self { id, label, address })
    }

    /// Returns the stable backend-owned identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the display label.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns an optional display address.
    pub fn address(&self) -> Option<&str> {
        self.address.as_deref()
    }
}

/// Validation failures for an [`IdentityOption`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityOptionError {
    /// The backend identity identifier was empty.
    EmptyId,
    /// The identity display label was empty.
    EmptyLabel,
}

impl fmt::Display for IdentityOptionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyId => formatter.write_str("identity identifier must not be empty"),
            Self::EmptyLabel => formatter.write_str("identity label must not be empty"),
        }
    }
}

impl std::error::Error for IdentityOptionError {}

/// A value that is only carried across the setup boundary and is never
/// displayed or formatted with its contents.
#[derive(Clone, Eq, PartialEq)]
struct SecretInput(String);

impl SecretInput {
    fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    fn is_blank(&self) -> bool {
        self.0.trim().is_empty()
    }

    fn is_present(&self) -> bool {
        !self.0.is_empty()
    }
}

impl fmt::Debug for SecretInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretInput")
            .field("present", &self.is_present())
            .finish()
    }
}

/// Activation and provisioning values submitted to the later backend seam.
#[derive(Clone, Eq, PartialEq)]
pub struct ActivationInput {
    device_label: String,
    activation_code: SecretInput,
    provisioning_code: SecretInput,
}

impl ActivationInput {
    /// Creates activation input. The code values are kept private and are
    /// never included in [`Debug`] output.
    pub fn new(
        device_label: impl Into<String>,
        activation_code: impl Into<String>,
        provisioning_code: impl Into<String>,
    ) -> Self {
        Self {
            device_label: device_label.into(),
            activation_code: SecretInput::new(activation_code),
            provisioning_code: SecretInput::new(provisioning_code),
        }
    }

    /// Returns the non-secret device label.
    pub fn device_label(&self) -> &str {
        &self.device_label
    }

    /// Returns whether an activation code was supplied.
    pub fn has_activation_code(&self) -> bool {
        self.activation_code.is_present()
    }

    /// Returns whether a provisioning code was supplied.
    pub fn has_provisioning_code(&self) -> bool {
        self.provisioning_code.is_present()
    }

    fn is_valid(&self) -> bool {
        !self.device_label.trim().is_empty()
            && !self.activation_code.is_blank()
            && !self.provisioning_code.is_blank()
    }
}

impl fmt::Debug for ActivationInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActivationInput")
            .field("device_label", &self.device_label)
            .field("activation_code", &"<redacted>")
            .field("provisioning_code", &"<redacted>")
            .finish()
    }
}

/// Account login values submitted to the later backend seam.
#[derive(Clone, Eq, PartialEq)]
pub struct AppleLoginInput {
    account: String,
    password: SecretInput,
}

impl AppleLoginInput {
    /// Creates login input. The password is kept private and is never included
    /// in [`Debug`] output.
    pub fn new(account: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            account: account.into(),
            password: SecretInput::new(password),
        }
    }

    /// Returns the account identifier entered by the user.
    pub fn account(&self) -> &str {
        &self.account
    }

    /// Returns whether a password was supplied.
    pub fn has_password(&self) -> bool {
        self.password.is_present()
    }

    fn is_valid(&self) -> bool {
        !self.account.trim().is_empty() && !self.password.is_blank()
    }
}

impl fmt::Debug for AppleLoginInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppleLoginInput")
            .field("account", &self.account)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// A second-factor code submitted to the later backend seam.
#[derive(Clone, Eq, PartialEq)]
pub struct TwoFactorInput {
    code: SecretInput,
}

impl TwoFactorInput {
    /// Creates a second-factor input. The code is never included in
    /// [`Debug`] output.
    pub fn new(code: impl Into<String>) -> Self {
        Self {
            code: SecretInput::new(code),
        }
    }

    /// Returns whether a code was supplied.
    pub fn has_code(&self) -> bool {
        self.code.is_present()
    }

    fn is_valid(&self) -> bool {
        let code = self.code.0.trim();
        (4..=8).contains(&code.len()) && code.chars().all(|character| character.is_ascii_digit())
    }
}

impl fmt::Debug for TwoFactorInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TwoFactorInput")
            .field("code", &"<redacted>")
            .finish()
    }
}

/// A synthetic/mockable update that a later daemon adapter can feed into the
/// setup reducer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackendUpdate {
    /// Provisioning accepted the submitted activation values.
    ProvisioningAccepted,
    /// Login requires a second factor.
    LoginRequiresTwoFactor,
    /// Login returned the identities that can be selected.
    LoginSucceeded { identities: Vec<IdentityOption> },
    /// Updates the connection state reported by the backend.
    ConnectionChanged(ConnectionState),
    /// Reports a safe, retryable backend failure.
    Failed(RecoverableError),
}

/// An event from the view or the backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SetupEvent {
    /// Starts setup from the first-run page.
    Begin,
    /// Returns to the previous user-input page.
    Back,
    /// Submits activation and provisioning inputs.
    SubmitActivation(ActivationInput),
    /// Submits account login inputs.
    SubmitAppleLogin(AppleLoginInput),
    /// Submits a second-factor code.
    SubmitTwoFactor(TwoFactorInput),
    /// Selects one identity returned by the backend.
    SelectIdentity(String),
    /// Retries the operation represented by the current recoverable error.
    Retry,
    /// Applies an update from the mocked/backend boundary.
    Backend(BackendUpdate),
}

/// A side effect for the future backend adapter. No effect performs I/O in
/// this module.
#[derive(Clone, Eq, PartialEq)]
pub enum SetupEffect {
    /// No backend work is requested.
    None,
    /// Begin activation/provisioning with the supplied values.
    Provision(ActivationInput),
    /// Begin account authentication with the supplied values.
    Authenticate(AppleLoginInput),
    /// Verify the supplied second-factor code.
    VerifyTwoFactor(TwoFactorInput),
    /// Apply the selected identity.
    ConfirmIdentity(String),
    /// Retry the connection after a recoverable connection failure.
    Reconnect,
}

impl fmt::Debug for SetupEffect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => formatter.write_str("None"),
            Self::Provision(input) => formatter.debug_tuple("Provision").field(input).finish(),
            Self::Authenticate(input) => {
                formatter.debug_tuple("Authenticate").field(input).finish()
            }
            Self::VerifyTwoFactor(input) => formatter
                .debug_tuple("VerifyTwoFactor")
                .field(input)
                .finish(),
            Self::ConfirmIdentity(identity) => formatter
                .debug_tuple("ConfirmIdentity")
                .field(identity)
                .finish(),
            Self::Reconnect => formatter.write_str("Reconnect"),
        }
    }
}

/// The result of applying one setup event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SetupTransition {
    /// The event was not valid for the current stage.
    Ignored,
    /// The event was accepted; the effect is sent to a later adapter.
    Applied(SetupEffect),
}

impl SetupTransition {
    /// Returns whether the event changed the state machine.
    pub const fn is_applied(&self) -> bool {
        matches!(self, Self::Applied(_))
    }
}

/// Pure setup/account state used by the view and by isolated tests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupState {
    stage: SetupStage,
    connection: ConnectionState,
    identities: Vec<IdentityOption>,
    selected_identity: Option<String>,
    error: Option<RecoverableError>,
    retry_stage: Option<SetupStage>,
}

impl Default for SetupState {
    fn default() -> Self {
        Self::new()
    }
}

impl SetupState {
    /// Creates a clean first-run state with no account or identity data.
    pub const fn new() -> Self {
        Self {
            stage: SetupStage::FirstRun,
            connection: ConnectionState::Disconnected,
            identities: Vec::new(),
            selected_identity: None,
            error: None,
            retry_stage: None,
        }
    }

    /// Returns the current page/stage.
    pub const fn stage(&self) -> SetupStage {
        self.stage
    }

    /// Returns the current connection state.
    pub const fn connection(&self) -> ConnectionState {
        self.connection
    }

    /// Returns the available identity options.
    pub fn identities(&self) -> &[IdentityOption] {
        &self.identities
    }

    /// Returns the selected identity identifier, if one has been selected.
    pub fn selected_identity(&self) -> Option<&str> {
        self.selected_identity.as_deref()
    }

    /// Returns the current recoverable error, if any.
    pub const fn error(&self) -> Option<RecoverableError> {
        self.error
    }

    /// Applies one event without performing I/O.
    pub fn transition(&mut self, event: SetupEvent) -> SetupTransition {
        match event {
            SetupEvent::Begin if self.stage == SetupStage::FirstRun => {
                self.move_to(SetupStage::Activation, ConnectionState::Disconnected, None);
                SetupTransition::Applied(SetupEffect::None)
            }
            SetupEvent::Back => self.back(),
            SetupEvent::SubmitActivation(input) if self.stage == SetupStage::Activation => {
                if input.is_valid() {
                    self.move_to(
                        SetupStage::Provisioning,
                        ConnectionState::Connecting,
                        Some(SetupStage::Activation),
                    );
                    SetupTransition::Applied(SetupEffect::Provision(input))
                } else {
                    self.show_error(
                        RecoverableError::InvalidActivationInput,
                        SetupStage::Activation,
                        ConnectionState::Disconnected,
                    );
                    SetupTransition::Applied(SetupEffect::None)
                }
            }
            SetupEvent::SubmitAppleLogin(input) if self.stage == SetupStage::AppleLogin => {
                if input.is_valid() {
                    self.move_to(
                        SetupStage::Connecting,
                        ConnectionState::Connecting,
                        Some(SetupStage::AppleLogin),
                    );
                    SetupTransition::Applied(SetupEffect::Authenticate(input))
                } else {
                    self.show_error(
                        RecoverableError::InvalidAppleLoginInput,
                        SetupStage::AppleLogin,
                        ConnectionState::Disconnected,
                    );
                    SetupTransition::Applied(SetupEffect::None)
                }
            }
            SetupEvent::SubmitTwoFactor(input) if self.stage == SetupStage::TwoFactor => {
                if input.is_valid() {
                    self.move_to(
                        SetupStage::Connecting,
                        ConnectionState::Connecting,
                        Some(SetupStage::TwoFactor),
                    );
                    SetupTransition::Applied(SetupEffect::VerifyTwoFactor(input))
                } else {
                    self.show_error(
                        RecoverableError::InvalidTwoFactorInput,
                        SetupStage::TwoFactor,
                        ConnectionState::Disconnected,
                    );
                    SetupTransition::Applied(SetupEffect::None)
                }
            }
            SetupEvent::SelectIdentity(identity_id)
                if self.stage == SetupStage::IdentitySelection =>
            {
                if self
                    .identities
                    .iter()
                    .any(|identity| identity.id() == identity_id)
                {
                    self.selected_identity = Some(identity_id.clone());
                    self.move_to(
                        SetupStage::Connecting,
                        ConnectionState::Connecting,
                        Some(SetupStage::IdentitySelection),
                    );
                    SetupTransition::Applied(SetupEffect::ConfirmIdentity(identity_id))
                } else {
                    self.show_error(
                        RecoverableError::IdentitySelectionFailed,
                        SetupStage::IdentitySelection,
                        ConnectionState::Connecting,
                    );
                    SetupTransition::Applied(SetupEffect::None)
                }
            }
            SetupEvent::Retry if self.stage == SetupStage::Error => self.retry(),
            SetupEvent::Backend(update) => self.apply_backend(update),
            _ => SetupTransition::Ignored,
        }
    }

    fn apply_backend(&mut self, update: BackendUpdate) -> SetupTransition {
        match update {
            BackendUpdate::ProvisioningAccepted if self.stage == SetupStage::Provisioning => {
                self.move_to(SetupStage::AppleLogin, ConnectionState::Disconnected, None);
                SetupTransition::Applied(SetupEffect::None)
            }
            BackendUpdate::LoginRequiresTwoFactor if self.stage == SetupStage::Connecting => {
                self.move_to(
                    SetupStage::TwoFactor,
                    ConnectionState::Connecting,
                    Some(SetupStage::TwoFactor),
                );
                SetupTransition::Applied(SetupEffect::None)
            }
            BackendUpdate::LoginSucceeded { identities }
                if self.stage == SetupStage::Connecting =>
            {
                if identities.is_empty() {
                    self.show_error(
                        RecoverableError::NoIdentityAvailable,
                        SetupStage::AppleLogin,
                        ConnectionState::Failed,
                    );
                    return SetupTransition::Applied(SetupEffect::None);
                }
                let mut ids = HashSet::with_capacity(identities.len());
                if identities.iter().any(|identity| !ids.insert(identity.id())) {
                    self.show_error(
                        RecoverableError::InvalidIdentityList,
                        SetupStage::AppleLogin,
                        ConnectionState::Failed,
                    );
                    return SetupTransition::Applied(SetupEffect::None);
                }
                self.identities = identities;
                self.selected_identity = None;
                self.move_to(
                    SetupStage::IdentitySelection,
                    ConnectionState::Connecting,
                    None,
                );
                SetupTransition::Applied(SetupEffect::None)
            }
            BackendUpdate::ConnectionChanged(ConnectionState::Connecting) => {
                self.connection = ConnectionState::Connecting;
                SetupTransition::Applied(SetupEffect::None)
            }
            BackendUpdate::ConnectionChanged(ConnectionState::Connected)
                if self.stage == SetupStage::Connecting && self.selected_identity.is_some() =>
            {
                self.move_to(SetupStage::Complete, ConnectionState::Connected, None);
                SetupTransition::Applied(SetupEffect::None)
            }
            BackendUpdate::ConnectionChanged(ConnectionState::Connected)
                if self.stage == SetupStage::Complete =>
            {
                self.connection = ConnectionState::Connected;
                SetupTransition::Applied(SetupEffect::None)
            }
            BackendUpdate::ConnectionChanged(ConnectionState::Disconnected)
                if matches!(
                    self.stage,
                    SetupStage::Provisioning
                        | SetupStage::Connecting
                        | SetupStage::TwoFactor
                        | SetupStage::Complete
                ) =>
            {
                let retry_stage = if self.stage == SetupStage::Complete {
                    SetupStage::Connecting
                } else {
                    self.retry_stage.unwrap_or(SetupStage::AppleLogin)
                };
                self.show_error(
                    RecoverableError::ConnectionLost,
                    retry_stage,
                    ConnectionState::Failed,
                );
                SetupTransition::Applied(SetupEffect::None)
            }
            BackendUpdate::ConnectionChanged(ConnectionState::Failed) => {
                let retry_stage = self.retry_target();
                self.show_error(
                    RecoverableError::BackendUnavailable,
                    retry_stage,
                    ConnectionState::Failed,
                );
                SetupTransition::Applied(SetupEffect::None)
            }
            BackendUpdate::Failed(error) => {
                let retry_stage = self.retry_target();
                self.show_error(error, retry_stage, ConnectionState::Failed);
                SetupTransition::Applied(SetupEffect::None)
            }
            _ => SetupTransition::Ignored,
        }
    }

    fn back(&mut self) -> SetupTransition {
        let target = match self.stage {
            SetupStage::Activation => Some(SetupStage::FirstRun),
            SetupStage::Provisioning => Some(SetupStage::Activation),
            SetupStage::AppleLogin => Some(SetupStage::Activation),
            SetupStage::TwoFactor => Some(SetupStage::AppleLogin),
            SetupStage::IdentitySelection => Some(SetupStage::AppleLogin),
            SetupStage::Connecting => Some(self.retry_stage.unwrap_or(SetupStage::AppleLogin)),
            SetupStage::Error => Some(self.retry_stage.unwrap_or(SetupStage::FirstRun)),
            SetupStage::FirstRun | SetupStage::Complete => None,
        };
        let Some(target) = target else {
            return SetupTransition::Ignored;
        };
        self.move_to(target, ConnectionState::Disconnected, None);
        if matches!(target, SetupStage::FirstRun | SetupStage::Activation) {
            self.identities.clear();
            self.selected_identity = None;
        }
        SetupTransition::Applied(SetupEffect::None)
    }

    fn retry(&mut self) -> SetupTransition {
        let target = self.retry_stage.unwrap_or(SetupStage::FirstRun);
        if target == SetupStage::Connecting {
            self.move_to(
                SetupStage::Connecting,
                ConnectionState::Connecting,
                Some(SetupStage::Connecting),
            );
            SetupTransition::Applied(SetupEffect::Reconnect)
        } else {
            self.move_to(target, ConnectionState::Disconnected, None);
            SetupTransition::Applied(SetupEffect::None)
        }
    }

    fn retry_target(&self) -> SetupStage {
        match self.stage {
            SetupStage::Provisioning => SetupStage::Activation,
            SetupStage::Connecting => self.retry_stage.unwrap_or(SetupStage::AppleLogin),
            SetupStage::TwoFactor => SetupStage::TwoFactor,
            SetupStage::IdentitySelection => SetupStage::IdentitySelection,
            SetupStage::Complete => SetupStage::Connecting,
            SetupStage::Error => self.retry_stage.unwrap_or(SetupStage::FirstRun),
            SetupStage::FirstRun | SetupStage::Activation | SetupStage::AppleLogin => self.stage,
        }
    }

    fn move_to(
        &mut self,
        stage: SetupStage,
        connection: ConnectionState,
        retry_stage: Option<SetupStage>,
    ) {
        self.stage = stage;
        self.connection = connection;
        self.error = None;
        self.retry_stage = retry_stage;
    }

    fn show_error(
        &mut self,
        error: RecoverableError,
        retry_stage: SetupStage,
        connection: ConnectionState,
    ) {
        self.stage = SetupStage::Error;
        self.connection = connection;
        self.error = Some(error);
        self.retry_stage = Some(retry_stage);
    }
}

type SetupListener = Box<dyn Fn(SetupState)>;

/// Shared state/controller handle for a [`SetupView`].
#[derive(Clone)]
pub struct SetupController {
    state: Rc<RefCell<SetupState>>,
    listeners: Rc<RefCell<Vec<SetupListener>>>,
}

impl Default for SetupController {
    fn default() -> Self {
        Self::new()
    }
}

impl SetupController {
    /// Creates a controller with a clean first-run state.
    pub fn new() -> Self {
        Self::with_state(SetupState::new())
    }

    /// Creates a controller around an explicit state, useful for previews and
    /// deterministic UI tests.
    pub fn with_state(state: SetupState) -> Self {
        Self {
            state: Rc::new(RefCell::new(state)),
            listeners: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// Returns a snapshot that can be inspected without holding a borrow.
    pub fn state(&self) -> SetupState {
        self.state.borrow().clone()
    }

    /// Applies an event and notifies all view listeners with a state snapshot.
    pub fn dispatch(&self, event: SetupEvent) -> SetupTransition {
        let transition = self.state.borrow_mut().transition(event);
        let snapshot = self.state();
        for listener in self.listeners.borrow().iter() {
            listener(snapshot.clone());
        }
        transition
    }

    /// Registers a callback invoked after each dispatched event.
    pub fn connect_changed<F>(&self, listener: F)
    where
        F: Fn(SetupState) + 'static,
    {
        self.listeners.borrow_mut().push(Box::new(listener));
    }
}

/// Reusable GTK4/libadwaita setup view.
///
/// The view can be embedded in a later application shell with
/// [`Self::widget`]. It does not create or own a daemon connection; callers
/// should observe the controller's effects and feed backend responses back as
/// [`SetupEvent::Backend`] events.
#[derive(Clone)]
pub struct SetupView {
    root: gtk::Box,
    controller: SetupController,
}

impl SetupView {
    /// Builds all setup pages and binds them to the supplied controller.
    pub fn new(controller: &SetupController) -> Self {
        let controller = controller.clone();
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        let header = adw::HeaderBar::new();
        let title = adw::WindowTitle::new("Set up LiteBubbles", "");
        header.set_title_widget(Some(&title));
        root.append(&header);

        let stack = gtk::Stack::builder()
            .hexpand(true)
            .vexpand(true)
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();

        let begin = gtk::Button::builder()
            .label("Begin setup")
            .css_classes(["suggested-action"])
            .halign(gtk::Align::Center)
            .build();
        let first_run = adw::StatusPage::builder()
            .icon_name("preferences-system-symbolic")
            .title("Set up LiteBubbles")
            .description("Connect an account to start using messages and services.")
            .build();
        first_run.set_child(Some(&begin));
        stack.add_named(&first_run, Some(SetupStage::FirstRun.page_name()));

        let (activation, device_entry, activation_entry, provisioning_entry, activation_submit) =
            activation_page();
        stack.add_named(&activation, Some(SetupStage::Activation.page_name()));

        let provisioning = status_page(
            "Provisioning device",
            "The setup service is preparing this device…",
            "view-refresh-symbolic",
        );
        stack.add_named(&provisioning, Some(SetupStage::Provisioning.page_name()));

        let (login, account_entry, password_entry, login_submit) = apple_login_page();
        stack.add_named(&login, Some(SetupStage::AppleLogin.page_name()));

        let (two_factor, two_factor_entry, two_factor_submit) = two_factor_page();
        stack.add_named(&two_factor, Some(SetupStage::TwoFactor.page_name()));

        let (identity_page, identity_list) = identity_selection_page();
        stack.add_named(
            &identity_page,
            Some(SetupStage::IdentitySelection.page_name()),
        );

        let connecting = status_page(
            "Connecting",
            "Finishing account setup…",
            "network-wireless-symbolic",
        );
        let connection_label = gtk::Label::builder()
            .halign(gtk::Align::Center)
            .margin_bottom(24)
            .build();
        connecting.append(&connection_label);
        stack.add_named(&connecting, Some(SetupStage::Connecting.page_name()));

        let complete = status_page(
            "Setup complete",
            "Your selected identity is connected and ready.",
            "emblem-ok-symbolic",
        );
        stack.add_named(&complete, Some(SetupStage::Complete.page_name()));

        let error_page = adw::StatusPage::builder()
            .icon_name("dialog-error-symbolic")
            .title("Setup needs attention")
            .description("You can retry this step.")
            .build();
        let error_actions = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .halign(gtk::Align::Center)
            .build();
        let error_back = gtk::Button::with_label("Back");
        let error_retry = gtk::Button::builder()
            .label("Try again")
            .css_classes(["suggested-action"])
            .build();
        error_actions.append(&error_back);
        error_actions.append(&error_retry);
        error_page.set_child(Some(&error_actions));
        stack.add_named(&error_page, Some(SetupStage::Error.page_name()));

        root.append(&stack);

        let controller_for_begin = controller.clone();
        begin.connect_clicked(move |_| {
            controller_for_begin.dispatch(SetupEvent::Begin);
        });

        let controller_for_activation = controller.clone();
        activation_submit.connect_clicked(move |_| {
            controller_for_activation.dispatch(SetupEvent::SubmitActivation(ActivationInput::new(
                device_entry.text(),
                activation_entry.text(),
                provisioning_entry.text(),
            )));
        });

        let controller_for_login = controller.clone();
        login_submit.connect_clicked(move |_| {
            controller_for_login.dispatch(SetupEvent::SubmitAppleLogin(AppleLoginInput::new(
                account_entry.text(),
                password_entry.text(),
            )));
        });

        let controller_for_two_factor = controller.clone();
        two_factor_submit.connect_clicked(move |_| {
            controller_for_two_factor.dispatch(SetupEvent::SubmitTwoFactor(TwoFactorInput::new(
                two_factor_entry.text(),
            )));
        });

        let controller_for_identity_list = controller.clone();
        identity_list.set_activate_on_single_click(true);
        identity_list.connect_row_activated(move |_, row| {
            let identity_id = controller_for_identity_list
                .state()
                .identities()
                .get(row.index() as usize)
                .map(|identity| identity.id().to_owned());
            if let Some(identity_id) = identity_id {
                controller_for_identity_list.dispatch(SetupEvent::SelectIdentity(identity_id));
            }
        });

        let controller_for_retry = controller.clone();
        error_retry.connect_clicked(move |_| {
            controller_for_retry.dispatch(SetupEvent::Retry);
        });
        let controller_for_back = controller.clone();
        error_back.connect_clicked(move |_| {
            controller_for_back.dispatch(SetupEvent::Back);
        });

        let stack_for_refresh = stack.clone();
        let identity_list_for_refresh = identity_list.clone();
        let error_page_for_refresh = error_page.clone();
        let connection_label_for_refresh = connection_label.clone();
        controller.connect_changed(move |state| {
            refresh_view(
                &state,
                &stack_for_refresh,
                &identity_list_for_refresh,
                &error_page_for_refresh,
                &connection_label_for_refresh,
            );
        });
        refresh_view(
            &controller.state(),
            &stack,
            &identity_list,
            &error_page,
            &connection_label,
        );

        Self { root, controller }
    }

    /// Returns the widget to embed in an application shell.
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Returns the controller used by this view.
    pub fn controller(&self) -> &SetupController {
        &self.controller
    }
}

fn status_page(title: &str, description: &str, icon_name: &str) -> gtk::Box {
    let page = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .valign(gtk::Align::Center)
        .vexpand(true)
        .build();
    let status = adw::StatusPage::builder()
        .icon_name(icon_name)
        .title(title)
        .description(description)
        .vexpand(false)
        .build();
    let spinner = gtk::Spinner::builder().spinning(true).build();
    status.set_child(Some(&spinner));
    page.append(&status);
    page
}

fn form_page(title: &str, description: &str, fields: &gtk::Box, submit: &gtk::Button) -> gtk::Box {
    let page = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .margin_start(24)
        .margin_end(24)
        .margin_top(24)
        .margin_bottom(24)
        .build();
    let heading = adw::StatusPage::builder()
        .title(title)
        .description(description)
        .vexpand(false)
        .build();
    page.append(&heading);
    page.append(fields);
    page.append(submit);
    page
}

fn action_row(title: &str, widget: &impl IsA<gtk::Widget>) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).build();
    row.add_suffix(widget);
    row
}

fn activation_page() -> (
    gtk::Box,
    gtk::Entry,
    gtk::PasswordEntry,
    gtk::PasswordEntry,
    gtk::Button,
) {
    let device_entry = gtk::Entry::builder()
        .placeholder_text("Device label")
        .hexpand(true)
        .build();
    let activation_entry = gtk::PasswordEntry::builder()
        .placeholder_text("Activation code")
        .hexpand(true)
        .build();
    let provisioning_entry = gtk::PasswordEntry::builder()
        .placeholder_text("Provisioning code")
        .hexpand(true)
        .build();
    let fields = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .build();
    fields.append(&action_row("Device", &device_entry));
    fields.append(&action_row("Activation", &activation_entry));
    fields.append(&action_row("Provisioning", &provisioning_entry));
    let submit = gtk::Button::builder()
        .label("Continue")
        .css_classes(["suggested-action"])
        .halign(gtk::Align::Center)
        .build();
    let page = form_page(
        "Activate this device",
        "Enter the setup values supplied by the backend administrator.",
        &fields,
        &submit,
    );
    (
        page,
        device_entry,
        activation_entry,
        provisioning_entry,
        submit,
    )
}

fn apple_login_page() -> (gtk::Box, gtk::Entry, gtk::PasswordEntry, gtk::Button) {
    let account_entry = gtk::Entry::builder()
        .placeholder_text("Account")
        .hexpand(true)
        .build();
    let password_entry = gtk::PasswordEntry::builder()
        .placeholder_text("Password")
        .hexpand(true)
        .build();
    let fields = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .build();
    fields.append(&action_row("Account", &account_entry));
    fields.append(&action_row("Password", &password_entry));
    let submit = gtk::Button::builder()
        .label("Sign in")
        .css_classes(["suggested-action"])
        .halign(gtk::Align::Center)
        .build();
    let page = form_page(
        "Sign in",
        "Continue with the account authorized for this device.",
        &fields,
        &submit,
    );
    (page, account_entry, password_entry, submit)
}

fn two_factor_page() -> (gtk::Box, gtk::Entry, gtk::Button) {
    let code_entry = gtk::Entry::builder()
        .placeholder_text("Verification code")
        .input_purpose(gtk::InputPurpose::Digits)
        .hexpand(true)
        .build();
    let fields = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .build();
    fields.append(&action_row("Code", &code_entry));
    let submit = gtk::Button::builder()
        .label("Verify")
        .css_classes(["suggested-action"])
        .halign(gtk::Align::Center)
        .build();
    let page = form_page(
        "Two-step verification",
        "Enter the numeric code supplied by the account provider.",
        &fields,
        &submit,
    );
    (page, code_entry, submit)
}

fn identity_selection_page() -> (gtk::Box, gtk::ListBox) {
    let page = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_start(24)
        .margin_end(24)
        .margin_top(24)
        .margin_bottom(24)
        .build();
    let heading = adw::StatusPage::builder()
        .title("Choose an identity")
        .description("Select the identity this device should use.")
        .vexpand(false)
        .build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .vexpand(true)
        .build();
    page.append(&heading);
    page.append(&list);
    (page, list)
}

fn refresh_view(
    state: &SetupState,
    stack: &gtk::Stack,
    identity_list: &gtk::ListBox,
    error_page: &adw::StatusPage,
    connection_label: &gtk::Label,
) {
    stack.set_visible_child_name(state.stage.page_name());
    connection_label.set_label(state.connection.label());
    if let Some(error) = state.error {
        error_page.set_title(error.title());
        error_page.set_description(Some(error.description()));
    }

    while let Some(child) = identity_list.first_child() {
        identity_list.remove(&child);
    }
    for identity in &state.identities {
        let row = adw::ActionRow::builder()
            .title(identity.label())
            .subtitle(identity.address().unwrap_or(""))
            .activatable(true)
            .build();
        identity_list.append(&row);
    }
}
