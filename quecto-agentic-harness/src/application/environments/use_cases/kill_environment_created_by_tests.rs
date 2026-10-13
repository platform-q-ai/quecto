//! `quecto container kill <ref> --created-by <key>`: the kill (or a
//! stopped environment's removal) goes ahead only on a record whose
//! CREATED-BY is exactly `<key>`, judged with the kill's own claim. A
//! mismatch is refused with its own error and nothing is asked, run or
//! changed.

use std::sync::{Arc, Mutex};

use crate::application::environments::dto::EnvironmentLiveness;
use crate::application::environments::ports::{
    EnvironmentMemberShutdown, EnvironmentProcessCommands, MemberShutdownReport, PortFuture,
};
use crate::application::environments::use_cases::{KillEnvironment, KillEnvironmentError};
use crate::domain::environments::entities::environment_registry::{
    ClaimCondition, CreatorMismatch, EnvironmentOrigin, EnvironmentRecord, EnvironmentRegistry,
    EnvironmentStatus, EnvironmentTarget,
};

/// Records every port call in order.
#[derive(Default)]
struct Spy {
    ran: Mutex<Vec<String>>,
}

impl Spy {
    fn push(&self, line: String) {
        self.ran.lock().unwrap().push(line);
    }

    fn ran(&self) -> Vec<String> {
        self.ran.lock().unwrap().clone()
    }
}

impl EnvironmentProcessCommands for Spy {
    fn run_retained_inspect<'a>(
        &'a self,
        _environment_id: &'a str,
        _argv: &'a [String],
    ) -> PortFuture<'a, Result<serde_json::Value, String>> {
        Box::pin(async { panic!("the kill never runs a post-mortem inspect") })
    }

    fn run_retained_kill<'a>(
        &'a self,
        environment_id: &'a str,
        _argv: &'a [String],
    ) -> PortFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.push(format!("kill {environment_id}"));
            Ok(())
        })
    }

    fn run_retained_cleanup<'a>(
        &'a self,
        environment_id: &'a str,
        _argv: &'a [String],
    ) -> PortFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.push(format!("cleanup {environment_id}"));
            Ok(())
        })
    }

    fn observe_liveness<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, EnvironmentLiveness> {
        Box::pin(async move {
            self.push(format!("liveness {}", record.environment_id));
            EnvironmentLiveness::Gone
        })
    }
}

struct Members(Arc<Spy>);

impl EnvironmentMemberShutdown for Members {
    fn shutdown_members<'a>(
        &'a self,
        members: &'a [String],
    ) -> PortFuture<'a, MemberShutdownReport> {
        Box::pin(async move {
            self.0.push(format!("members({})", members.join(",")));
            MemberShutdownReport::default()
        })
    }
}

const POCKET: &str = "cli:pocket-7";

fn record(env_ref: &str, status: EnvironmentStatus, created_by: &str) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: env_ref.to_string(),
        environment_id: format!("env-{env_ref}"),
        environment_uuid: format!("uuid-{env_ref}"),
        name: None,
        workspace_path: std::path::PathBuf::from(format!("/state/env-{env_ref}/workspace")),
        repository: "https://example.invalid/repo.git".to_string(),
        script_name: "standard".to_string(),
        retained_exec_argv: vec!["exec.sh".to_string()],
        retained_kill_argv: vec!["kill.sh".to_string()],
        retained_cleanup_argv: vec!["cleanup.sh".to_string()],
        retained_inspect_argv: vec!["inspect.sh".to_string()],
        members: vec![],
        status,
        metadata: serde_json::json!({}),
        last_error: None,
        origin: EnvironmentOrigin::Restored,
        created_by: created_by.to_string(),
        created_at: None,
    }
}

fn kill_if(
    registry: &EnvironmentRegistry,
    spy: &Arc<Spy>,
    target: EnvironmentTarget,
    condition: ClaimCondition,
) -> Result<crate::application::environments::use_cases::KilledEnvironment, KillEnvironmentError> {
    let use_case = KillEnvironment::new(
        registry.clone(),
        Arc::new(Members(spy.clone())),
        spy.clone(),
        Arc::new(super::finalize_environment_member_tests::NoHostedStore),
    );
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(use_case.kill_container_if(&target, &condition))
}

fn by_ref(env_ref: &str) -> EnvironmentTarget {
    EnvironmentTarget::Ref(env_ref.to_string())
}

#[test]
fn a_matching_creator_kills_a_live_environment() {
    let registry = EnvironmentRegistry::new();
    registry.commit(record("C1", EnvironmentStatus::Running, POCKET));
    let spy = Arc::new(Spy::default());
    let killed = kill_if(
        &registry,
        &spy,
        by_ref("C1"),
        ClaimCondition::CreatedBy(POCKET.into()),
    )
    .unwrap();
    assert!(!killed.removed_stopped);
    assert_eq!(spy.ran(), ["members()", "kill env-C1"]);
    assert_eq!(
        registry.get("C1").unwrap().status,
        EnvironmentStatus::Stopped
    );
}

#[test]
fn another_creators_live_environment_is_refused_untouched() {
    for status in [
        EnvironmentStatus::Running,
        EnvironmentStatus::Retained,
        EnvironmentStatus::CleanupFailed,
    ] {
        let registry = EnvironmentRegistry::new();
        registry.commit(record("C1", status.clone(), "cli:pocket-70"));
        let spy = Arc::new(Spy::default());
        let error = kill_if(
            &registry,
            &spy,
            by_ref("C1"),
            ClaimCondition::CreatedBy(POCKET.into()),
        )
        .unwrap_err();
        assert_eq!(
            error,
            KillEnvironmentError::NotCreatedBy(CreatorMismatch {
                environment_ref: "C1".into(),
                expected: POCKET.into(),
                created_by: "cli:pocket-70".into(),
            })
        );
        assert!(spy.ran().is_empty(), "nothing was asked or run");
        assert_eq!(registry.get("C1").unwrap().status, status);
    }
}

#[test]
fn another_creators_stopped_environment_is_not_removed() {
    let registry = EnvironmentRegistry::new();
    registry.commit(record("C1", EnvironmentStatus::Stopped, "cli:other"));
    let spy = Arc::new(Spy::default());
    let error = kill_if(
        &registry,
        &spy,
        by_ref("C1"),
        ClaimCondition::CreatedBy(POCKET.into()),
    )
    .unwrap_err();
    assert!(
        matches!(error, KillEnvironmentError::NotCreatedBy(_)),
        "{error:?}"
    );
    assert!(spy.ran().is_empty(), "no liveness asked, no cleanup run");
    assert_eq!(
        registry.get("C1").unwrap().status,
        EnvironmentStatus::Stopped
    );
}

#[test]
fn a_matching_creators_stopped_environment_is_removed() {
    let registry = EnvironmentRegistry::new();
    registry.commit(record("C1", EnvironmentStatus::Stopped, POCKET));
    let spy = Arc::new(Spy::default());
    let killed = kill_if(
        &registry,
        &spy,
        by_ref("C1"),
        ClaimCondition::CreatedBy(POCKET.into()),
    )
    .unwrap();
    assert!(killed.removed_stopped);
    assert_eq!(spy.ran(), ["liveness env-C1", "cleanup env-C1"]);
    assert!(registry.get("C1").is_none(), "forgotten");
}

/// The creator is judged before any other refusal, so a caller learns the
/// record is not its own rather than why it could not be killed.
#[test]
fn a_mismatch_is_reported_before_the_scripts_own_refusals() {
    let registry = EnvironmentRegistry::new();
    let mut no_kill = record("C1", EnvironmentStatus::Running, "cli:other");
    no_kill.retained_kill_argv.clear();
    registry.commit(no_kill);
    let spy = Arc::new(Spy::default());
    let error = kill_if(
        &registry,
        &spy,
        by_ref("C1"),
        ClaimCondition::CreatedBy(POCKET.into()),
    )
    .unwrap_err();
    assert!(
        matches!(error, KillEnvironmentError::NotCreatedBy(_)),
        "{error:?}"
    );
}

#[test]
fn the_refusal_names_both_creators_and_that_nothing_was_killed() {
    let error = KillEnvironmentError::NotCreatedBy(CreatorMismatch {
        environment_ref: "C1".into(),
        expected: POCKET.into(),
        created_by: "cli:other".into(),
    });
    assert_eq!(
        error.to_string(),
        "environment C1 was created by cli:other, not cli:pocket-7; nothing was killed"
    );
    let unnamed = KillEnvironmentError::NotCreatedBy(CreatorMismatch {
        environment_ref: "C1".into(),
        expected: POCKET.into(),
        created_by: String::new(),
    });
    assert_eq!(
        unnamed.to_string(),
        "environment C1 was created by no session, not cli:pocket-7; nothing was killed"
    );
}

#[test]
fn an_unconditional_kill_is_unchanged() {
    let registry = EnvironmentRegistry::new();
    registry.commit(record("C1", EnvironmentStatus::Running, "cli:other"));
    let spy = Arc::new(Spy::default());
    kill_if(&registry, &spy, by_ref("C1"), ClaimCondition::Any).unwrap();
    assert_eq!(spy.ran(), ["members()", "kill env-C1"]);
}
