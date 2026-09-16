use super::{ResumeEndpointAction, ResumeEndpointRequest};

#[test]
fn request_preserves_the_exact_opaque_persisted_key() {
    let request = ResumeEndpointRequest {
        persisted_key: " padded:Key ".to_owned(),
        action: ResumeEndpointAction::ForkCurrent,
        location: None,
    };

    assert_eq!(request.persisted_key, " padded:Key ");
}

#[test]
fn endpoint_actions_are_explicit_and_affirmative() {
    let actions = [
        ResumeEndpointAction::OpenOriginal,
        ResumeEndpointAction::ForkCurrent,
        ResumeEndpointAction::Locate,
        ResumeEndpointAction::Cancel,
    ];

    assert_eq!(actions.len(), 4);
}
