#[allow(dead_code)]
#[path = "../src/setup.rs"]
mod setup;

use setup::{
    ActivationInput, BackendUpdate, ConnectionState, IdentityOption, RecoverableError, SetupEffect,
    SetupEvent, SetupStage, SetupState, SetupTransition, TwoFactorInput,
};

fn synthetic_identity(id: &str, label: &str) -> IdentityOption {
    IdentityOption::new(id, label, Some(format!("{id}@placeholder.invalid")))
        .expect("synthetic identity is valid")
}

fn begin_activation(state: &mut SetupState) {
    assert_eq!(
        state.transition(SetupEvent::Begin),
        SetupTransition::Applied(SetupEffect::None)
    );
}

fn provision(state: &mut SetupState) {
    let transition = state.transition(SetupEvent::SubmitActivation(ActivationInput::new(
        "fixture-device",
        "synthetic-activation",
        "synthetic-provisioning",
    )));
    assert!(
        matches!(transition, SetupTransition::Applied(SetupEffect::Provision(input))
        if input.device_label() == "fixture-device"
            && input.has_activation_code()
            && input.has_provisioning_code())
    );
    assert_eq!(state.stage(), SetupStage::Provisioning);
    assert_eq!(state.connection(), ConnectionState::Connecting);
    assert_eq!(
        state.transition(SetupEvent::Backend(BackendUpdate::ProvisioningAccepted)),
        SetupTransition::Applied(SetupEffect::None)
    );
    assert_eq!(state.stage(), SetupStage::AppleLogin);
}

#[test]
fn starts_clean_and_enters_activation() {
    let mut state = SetupState::new();

    assert_eq!(state.stage(), SetupStage::FirstRun);
    assert_eq!(state.connection(), ConnectionState::Disconnected);
    assert!(state.identities().is_empty());
    assert_eq!(state.error(), None);

    begin_activation(&mut state);
    assert_eq!(state.stage(), SetupStage::Activation);
    assert_eq!(state.connection(), ConnectionState::Disconnected);
}

#[test]
fn activation_validation_is_recoverable() {
    let mut state = SetupState::new();
    begin_activation(&mut state);

    let transition = state.transition(SetupEvent::SubmitActivation(ActivationInput::new(
        "fixture-device",
        "",
        "synthetic-provisioning",
    )));
    assert_eq!(transition, SetupTransition::Applied(SetupEffect::None));
    assert_eq!(state.stage(), SetupStage::Error);
    assert_eq!(
        state.error(),
        Some(RecoverableError::InvalidActivationInput)
    );
    assert_eq!(state.connection(), ConnectionState::Disconnected);

    assert_eq!(
        state.transition(SetupEvent::Retry),
        SetupTransition::Applied(SetupEffect::None)
    );
    assert_eq!(state.stage(), SetupStage::Activation);
    assert_eq!(state.error(), None);
}

#[test]
fn complete_flow_covers_login_two_factor_identity_and_connection() {
    let mut state = SetupState::new();
    begin_activation(&mut state);
    provision(&mut state);

    let login_transition = state.transition(SetupEvent::SubmitAppleLogin(
        setup::AppleLoginInput::new("fixture-account", "synthetic-password"),
    ));
    assert!(matches!(
        login_transition,
        SetupTransition::Applied(SetupEffect::Authenticate(input))
            if input.account() == "fixture-account" && input.has_password()
    ));
    assert_eq!(state.stage(), SetupStage::Connecting);

    assert_eq!(
        state.transition(SetupEvent::Backend(BackendUpdate::LoginRequiresTwoFactor)),
        SetupTransition::Applied(SetupEffect::None)
    );
    assert_eq!(state.stage(), SetupStage::TwoFactor);

    let two_factor_transition =
        state.transition(SetupEvent::SubmitTwoFactor(TwoFactorInput::new("123456")));
    assert!(matches!(
        two_factor_transition,
        SetupTransition::Applied(SetupEffect::VerifyTwoFactor(input)) if input.has_code()
    ));
    assert_eq!(state.stage(), SetupStage::Connecting);

    let identities = vec![
        synthetic_identity("identity-one", "Fixture one"),
        synthetic_identity("identity-two", "Fixture two"),
    ];
    assert_eq!(
        state.transition(SetupEvent::Backend(BackendUpdate::LoginSucceeded {
            identities
        })),
        SetupTransition::Applied(SetupEffect::None)
    );
    assert_eq!(state.stage(), SetupStage::IdentitySelection);
    assert_eq!(state.identities().len(), 2);
    assert_eq!(state.connection(), ConnectionState::Connecting);

    assert!(matches!(
        state.transition(SetupEvent::SelectIdentity("identity-two".to_owned())),
        SetupTransition::Applied(SetupEffect::ConfirmIdentity(identity))
            if identity == "identity-two"
    ));
    assert_eq!(state.stage(), SetupStage::Connecting);
    assert_eq!(state.selected_identity(), Some("identity-two"));

    assert_eq!(
        state.transition(SetupEvent::Backend(BackendUpdate::ConnectionChanged(
            ConnectionState::Connected,
        ))),
        SetupTransition::Applied(SetupEffect::None)
    );
    assert_eq!(state.stage(), SetupStage::Complete);
    assert_eq!(state.connection(), ConnectionState::Connected);
    assert_eq!(state.error(), None);
}

#[test]
fn direct_login_can_skip_two_factor_when_backend_allows_it() {
    let mut state = SetupState::new();
    begin_activation(&mut state);
    provision(&mut state);
    state.transition(SetupEvent::SubmitAppleLogin(setup::AppleLoginInput::new(
        "fixture-account",
        "synthetic-password",
    )));

    let identities = vec![synthetic_identity("identity-one", "Fixture one")];
    state.transition(SetupEvent::Backend(BackendUpdate::LoginSucceeded {
        identities,
    }));
    assert_eq!(state.stage(), SetupStage::IdentitySelection);
}

#[test]
fn invalid_identity_selection_and_identity_list_are_recoverable() {
    let mut state = SetupState::new();
    begin_activation(&mut state);
    provision(&mut state);
    state.transition(SetupEvent::SubmitAppleLogin(setup::AppleLoginInput::new(
        "fixture-account",
        "synthetic-password",
    )));
    state.transition(SetupEvent::Backend(BackendUpdate::LoginSucceeded {
        identities: vec![synthetic_identity("identity-one", "Fixture one")],
    }));

    state.transition(SetupEvent::SelectIdentity("unknown".to_owned()));
    assert_eq!(state.stage(), SetupStage::Error);
    assert_eq!(
        state.error(),
        Some(RecoverableError::IdentitySelectionFailed)
    );
    state.transition(SetupEvent::Retry);
    assert_eq!(state.stage(), SetupStage::IdentitySelection);

    state.transition(SetupEvent::Back);
    state.transition(SetupEvent::SubmitAppleLogin(setup::AppleLoginInput::new(
        "fixture-account",
        "synthetic-password",
    )));
    state.transition(SetupEvent::Backend(BackendUpdate::LoginSucceeded {
        identities: vec![
            synthetic_identity("duplicate", "Fixture duplicate one"),
            synthetic_identity("duplicate", "Fixture duplicate two"),
        ],
    }));
    assert_eq!(state.stage(), SetupStage::Error);
    assert_eq!(state.error(), Some(RecoverableError::InvalidIdentityList));
}

#[test]
fn backend_failures_and_connection_loss_can_retry() {
    let mut state = SetupState::new();
    begin_activation(&mut state);
    provision(&mut state);
    state.transition(SetupEvent::SubmitAppleLogin(setup::AppleLoginInput::new(
        "fixture-account",
        "synthetic-password",
    )));

    state.transition(SetupEvent::Backend(BackendUpdate::Failed(
        RecoverableError::AuthenticationFailed,
    )));
    assert_eq!(state.stage(), SetupStage::Error);
    assert_eq!(state.error(), Some(RecoverableError::AuthenticationFailed));
    assert_eq!(
        state.transition(SetupEvent::Retry),
        SetupTransition::Applied(SetupEffect::None)
    );
    assert_eq!(state.stage(), SetupStage::AppleLogin);

    state.transition(SetupEvent::SubmitAppleLogin(setup::AppleLoginInput::new(
        "fixture-account",
        "synthetic-password",
    )));
    state.transition(SetupEvent::Backend(BackendUpdate::LoginSucceeded {
        identities: vec![synthetic_identity("identity-one", "Fixture one")],
    }));
    state.transition(SetupEvent::SelectIdentity("identity-one".to_owned()));
    state.transition(SetupEvent::Backend(BackendUpdate::ConnectionChanged(
        ConnectionState::Connected,
    )));
    assert_eq!(state.stage(), SetupStage::Complete);

    state.transition(SetupEvent::Backend(BackendUpdate::ConnectionChanged(
        ConnectionState::Disconnected,
    )));
    assert_eq!(state.stage(), SetupStage::Error);
    assert_eq!(state.error(), Some(RecoverableError::ConnectionLost));
    assert_eq!(
        state.transition(SetupEvent::Retry),
        SetupTransition::Applied(SetupEffect::Reconnect)
    );
    assert_eq!(state.stage(), SetupStage::Connecting);
    assert_eq!(state.connection(), ConnectionState::Connecting);
}

#[test]
fn invalid_login_and_two_factor_inputs_are_redacted_and_recoverable() {
    let mut state = SetupState::new();
    begin_activation(&mut state);
    provision(&mut state);

    let login = setup::AppleLoginInput::new("fixture-account", "secret-login-placeholder");
    let login_debug = format!("{login:?}");
    assert!(!login_debug.contains("secret-login-placeholder"));
    assert!(login_debug.contains("<redacted>"));
    state.transition(SetupEvent::SubmitAppleLogin(login));
    state.transition(SetupEvent::Backend(BackendUpdate::LoginRequiresTwoFactor));

    let code = TwoFactorInput::new("not-a-code");
    let code_debug = format!("{code:?}");
    assert!(!code_debug.contains("not-a-code"));
    state.transition(SetupEvent::SubmitTwoFactor(code));
    assert_eq!(state.stage(), SetupStage::Error);
    assert_eq!(state.error(), Some(RecoverableError::InvalidTwoFactorInput));
}

#[test]
fn controller_notifies_with_snapshots_without_needing_gtk_main_wiring() {
    let controller = setup::SetupController::new();
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen_by_listener = seen.clone();
    controller.connect_changed(move |state| {
        seen_by_listener.borrow_mut().push(state.stage());
    });

    controller.dispatch(SetupEvent::Begin);
    assert_eq!(&*seen.borrow(), &[SetupStage::Activation]);
    assert_eq!(controller.state().stage(), SetupStage::Activation);
}

#[test]
fn controller_emits_backend_effects_without_leaking_setup_secrets() {
    let controller = setup::SetupController::new();
    let effects = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let effects_for_listener = effects.clone();
    controller.connect_effect(move |effect| {
        effects_for_listener
            .borrow_mut()
            .push(format!("{effect:?}"));
    });

    controller.dispatch(SetupEvent::Begin);
    controller.dispatch(SetupEvent::SubmitActivation(ActivationInput::new(
        "fixture-device",
        "activation-secret-placeholder",
        "provisioning-secret-placeholder",
    )));

    let effects = effects.borrow();
    assert_eq!(effects.len(), 2);
    assert_eq!(effects[0], "None");
    assert!(effects[1].starts_with("Provision(ActivationInput"));
    assert!(!effects[1].contains("activation-secret-placeholder"));
    assert!(!effects[1].contains("provisioning-secret-placeholder"));
}

#[test]
fn irrelevant_events_are_ignored_without_mutating_state() {
    let mut state = SetupState::new();
    let before = state.clone();
    assert_eq!(
        state.transition(SetupEvent::Retry),
        SetupTransition::Ignored
    );
    assert_eq!(
        state.transition(SetupEvent::Backend(BackendUpdate::ConnectionChanged(
            ConnectionState::Connected,
        ))),
        SetupTransition::Ignored
    );
    assert_eq!(state, before);
}
