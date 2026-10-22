//! Contract for [`KeptEnvironments`] (#2681), proven on the adapter over the
//! session's environment registry: the memberships of a teardown's agents
//! are the live environments any of them is a member of, each once; after
//! the teardown, an environment it retained is reported with the reason its
//! record holds, in the order asked, and one it ended (or no longer knows)
//! is left out.
use quecto::application::subagents::ports::kept_environments::{KeptEnvironment, KeptEnvironments};
use quecto::domain::environments::entities::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};
use quecto::domain::identity::ids::AgentUuid;
use quecto::infrastructure::tools::agents::kept_environments::RegistryKeptEnvironments;

fn record(env_ref: &str, members: &[&str], status: EnvironmentStatus) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: env_ref.into(),
        environment_id: format!("env-{env_ref}"),
        environment_uuid: format!("uuid-{env_ref}"),
        name: None,
        workspace_path: format!("/state/env-{env_ref}/workspace").into(),
        repository: String::new(),
        script_name: "standard".into(),
        retained_exec_argv: vec!["exec.sh".into()],
        retained_kill_argv: vec!["kill.sh".into()],
        retained_cleanup_argv: vec![],
        retained_inspect_argv: vec![],
        members: members.iter().map(|member| (*member).to_owned()).collect(),
        status,
        metadata: serde_json::json!({}),
        last_error: None,
        origin: EnvironmentOrigin::Created,
        created_by: String::new(),
        created_at: None,
    }
}

fn agents(uuids: &[&str]) -> Vec<AgentUuid> {
    uuids.iter().map(|uuid| AgentUuid::new(*uuid)).collect()
}

/// A registry holding `C1` (members A, B), `C2` (member C), `C3` (member
/// D, already retained) and `C4` (no members).
fn registry() -> EnvironmentRegistry {
    let registry = EnvironmentRegistry::new();
    registry.commit(record("C1", &["A", "B"], EnvironmentStatus::Running));
    registry.commit(record("C2", &["C"], EnvironmentStatus::Running));
    registry.commit(record("C3", &["D"], EnvironmentStatus::Retained));
    registry.commit(record("C4", &[], EnvironmentStatus::Running));
    registry
}

#[test]
fn memberships_are_the_live_environments_any_agent_is_a_member_of_each_once() {
    let port = RegistryKeptEnvironments::new(registry());
    let mut found = port.memberships(&agents(&["A", "B", "C", "D", "Z"]));
    found.sort();
    assert_eq!(found, ["C1", "C2"], "C3 was kept before; Z is in none");
    assert!(port.memberships(&agents(&["Z"])).is_empty());
    assert!(port.memberships(&[]).is_empty());
}

#[test]
fn kept_reports_each_retained_environment_with_its_records_reason() {
    let registry = registry();
    let port = RegistryKeptEnvironments::new(registry.clone());
    let asked = port.memberships(&agents(&["A", "B", "C"]));
    // The teardown: C1's last member leaves and its box is kept; C2's is
    // ended by its retained kill.
    registry.remove_member("C1", "A").unwrap();
    let claim = registry
        .remove_member("C1", "B")
        .unwrap()
        .expect("the last member's removal claims the teardown");
    registry.retain(claim, "kept so its work can be pushed from its checkout");
    let claim = registry.remove_member("C2", "C").unwrap().unwrap();
    registry.complete_kill(claim);

    assert_eq!(
        port.kept(&asked),
        [KeptEnvironment {
            environment_ref: "C1".into(),
            reason: "kept so its work can be pushed from its checkout".into(),
        }]
    );
    assert!(port.kept(&["C9".to_owned()]).is_empty(), "unknown");
    assert!(port.kept(&[]).is_empty());
}
