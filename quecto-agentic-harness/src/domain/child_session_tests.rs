use super::*;

#[test]
fn a_launched_child_runs_as_its_cli_session() {
    let child = AgentUuid::new("65268567-be4a-471f-a805-1238dcf08b68");
    assert_eq!(
        child_session_identity(&child).unwrap().runtime_key(),
        "cli:65268567-be4a-471f-a805-1238dcf08b68"
    );
    assert!(child_session_identity(&AgentUuid::new("a b")).is_err());
}

#[test]
fn a_childs_session_key_is_its_runtime_key_or_empty() {
    assert_eq!(child_runtime_key(&AgentUuid::new("b")), "cli:b");
    assert_eq!(child_runtime_key(&AgentUuid::new("a b")), "");
}

#[test]
fn a_childs_session_name_is_its_uuid_and_names_its_identity() {
    let child = AgentUuid::new("b");
    assert_eq!(child_session_name(&child), "b");
    assert_eq!(
        child_session_identity(&child).unwrap(),
        SessionIdentity::named_cli(child_session_name(&child)).unwrap()
    );
}

/// #2192 review round 5 (M2): only a launched child's roster row names its
/// session.
#[test]
fn only_a_launched_childs_roster_row_names_its_session() {
    use crate::domain::child_end::ChildOrigin;
    let child = AgentUuid::new("secret-plan");
    assert_eq!(
        roster_session_key(&child, ChildOrigin::Launched),
        "cli:secret-plan"
    );
    assert_eq!(roster_session_key(&child, ChildOrigin::Reported), "");
    assert_eq!(roster_session_key(&child, ChildOrigin::Unverified), "");
}
