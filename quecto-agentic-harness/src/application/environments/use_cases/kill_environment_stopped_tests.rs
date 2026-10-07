//! #2206: `kill_container` (and `quecto container kill`) of a `stopped`
//! environment removes what it left — the retained `cleanup` takes the
//! exited container and the state directory — and forgets the record. It
//! is refused, untouched, unless the runtime affirms the container is gone:
//! a box another live session still runs is never destroyed.

use std::sync::{Arc, Mutex};

use crate::application::environments::dto::EnvironmentLiveness;
use crate::application::environments::ports::{
    EnvironmentMemberShutdown, EnvironmentProcessCommands, HostedSwarmRunObservation,
    MemberShutdownReport, PortFuture,
};
use crate::application::environments::use_cases::{KillEnvironment, KillEnvironmentError};
use crate::domain::environments::entities::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus, EnvironmentTarget,
};
use crate::domain::environments::services::environment_retention::{
    CoordinatorLoss, HostedSwarmRun, SwarmRunObservation,
};
use crate::domain::swarm::RunStatus;

/// Records every script, answers liveness as configured.
struct Commands {
    liveness: EnvironmentLiveness,
    cleanup_error: Option<String>,
    /// How many cleanups fail with `cleanup_error` before one succeeds.
    cleanup_failures: Mutex<usize>,
    ran: Mutex<Vec<String>>,
}

impl Commands {
    fn answering(liveness: EnvironmentLiveness) -> Arc<Self> {
        Arc::new(Self {
            liveness,
            cleanup_error: None,
            cleanup_failures: Mutex::new(0),
            ran: Mutex::new(Vec::new()),
        })
    }

    fn ran(&self) -> Vec<String> {
        self.ran.lock().unwrap().clone()
    }
}

impl EnvironmentProcessCommands for Commands {
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
            self.ran
                .lock()
                .unwrap()
                .push(format!("kill {environment_id}"));
            Ok(())
        })
    }

    fn run_retained_cleanup<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> PortFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.ran
                .lock()
                .unwrap()
                .push(format!("cleanup {environment_id} {}", argv.join(" ")));
            let mut failures = self.cleanup_failures.lock().unwrap();
            match (&self.cleanup_error, *failures) {
                (Some(error), 1..) => {
                    *failures -= 1;
                    Err(error.clone())
                }
                _ => Ok(()),
            }
        })
    }

    fn observe_liveness<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, EnvironmentLiveness> {
        Box::pin(async move {
            self.ran
                .lock()
                .unwrap()
                .push(format!("liveness {}", record.environment_id));
            self.liveness.clone()
        })
    }
}

/// A stopped record has no members; one asked about any is a bug.
struct NoMembers;

impl EnvironmentMemberShutdown for NoMembers {
    fn shutdown_members<'a>(
        &'a self,
        members: &'a [String],
    ) -> PortFuture<'a, MemberShutdownReport> {
        Box::pin(async move {
            assert!(members.is_empty(), "a stopped environment has no members");
            MemberShutdownReport::default()
        })
    }
}

fn record(
    env_ref: &str,
    status: EnvironmentStatus,
    origin: EnvironmentOrigin,
) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: env_ref.to_string(),
        environment_id: format!("env-{env_ref}"),
        environment_uuid: format!("uuid-{env_ref}"),
        name: Some("box".to_string()),
        workspace_path: std::path::PathBuf::from(format!("/state/env-{env_ref}/workspace")),
        repository: "https://example.invalid/repo.git".to_string(),
        script_name: "standard".to_string(),
        retained_exec_argv: vec!["exec.sh".to_string()],
        retained_kill_argv: vec!["kill.sh".to_string(), "--op".into(), "kill".into()],
        retained_cleanup_argv: vec!["kill.sh".to_string(), "--op".into(), "cleanup".into()],
        retained_inspect_argv: vec!["inspect.sh".to_string()],
        members: vec![],
        status,
        metadata: serde_json::json!({}),
        last_error: None,
        origin,
        created_by: "cli:other".to_string(),
        created_at: None,
    }
}

/// Reports the swarm run the checkout hosts, as configured.
struct Hosting(SwarmRunObservation);

impl HostedSwarmRunObservation for Hosting {
    fn observe_hosted_swarm_run<'a>(
        &'a self,
        _record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, SwarmRunObservation> {
        Box::pin(async move { self.0.clone() })
    }

    fn record_lost_coordinator<'a>(
        &'a self,
        _record: &'a EnvironmentRecord,
        _hosted: &'a HostedSwarmRun,
    ) -> PortFuture<'a, Result<CoordinatorLoss, String>> {
        Box::pin(async { panic!("a kill records no loss") })
    }
}

fn kill(
    registry: &EnvironmentRegistry,
    commands: &Arc<Commands>,
    target: EnvironmentTarget,
) -> Result<crate::application::environments::use_cases::KilledEnvironment, KillEnvironmentError> {
    kill_hosting(registry, commands, target, SwarmRunObservation::NoStore)
}

fn kill_hosting(
    registry: &EnvironmentRegistry,
    commands: &Arc<Commands>,
    target: EnvironmentTarget,
    hosted: SwarmRunObservation,
) -> Result<crate::application::environments::use_cases::KilledEnvironment, KillEnvironmentError> {
    let use_case = KillEnvironment::new(
        registry.clone(),
        Arc::new(NoMembers),
        commands.clone(),
        Arc::new(Hosting(hosted)),
    );
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(use_case.kill_container(&target))
}

fn by_ref(env_ref: &str) -> EnvironmentTarget {
    EnvironmentTarget::Ref(env_ref.to_string())
}

#[test]
fn a_stopped_environment_whose_container_is_gone_is_removed_and_forgotten() {
    for origin in [EnvironmentOrigin::Created, EnvironmentOrigin::Restored] {
        let registry = EnvironmentRegistry::new();
        registry.commit(record("C3", EnvironmentStatus::Stopped, origin));
        let commands = Commands::answering(EnvironmentLiveness::Gone);

        let removed = kill(&registry, &commands, by_ref("C3")).expect("removed");

        assert!(removed.removed_stopped, "{origin:?}");
        assert_eq!(removed.record.environment_ref, "C3");
        assert_eq!(
            commands.ran(),
            ["liveness env-C3", "cleanup env-C3 kill.sh --op cleanup"],
            "{origin:?}: liveness first, then the retained cleanup once, never the kill"
        );
        assert!(registry.get("C3").is_none(), "{origin:?}: forgotten");
    }
}

#[test]
fn a_stopped_environment_of_another_live_session_whose_box_runs_is_refused_untouched() {
    let registry = EnvironmentRegistry::new();
    registry.commit(record(
        "C4",
        EnvironmentStatus::Stopped,
        EnvironmentOrigin::Restored,
    ));
    let commands = Commands::answering(EnvironmentLiveness::Running);

    let error = kill(&registry, &commands, by_ref("C4")).unwrap_err();

    let KillEnvironmentError::Refused(detail) = error else {
        panic!("refused before any claim: {error:?}");
    };
    assert!(
        detail.contains("is stopped in the registry, but its container is running"),
        "{detail}"
    );
    assert!(detail.contains("nothing was removed"), "{detail}");
    assert_eq!(commands.ran(), ["liveness env-C4"], "no script ran");
    let record = registry.get("C4").expect("kept");
    assert_eq!(record.status, EnvironmentStatus::Stopped, "no claim taken");
    assert!(record.last_error.is_none());
}

#[test]
fn a_stopped_environment_whose_liveness_is_unknown_is_refused_untouched() {
    let registry = EnvironmentRegistry::new();
    registry.commit(record(
        "C5",
        EnvironmentStatus::Stopped,
        EnvironmentOrigin::Restored,
    ));
    let commands = Commands::answering(EnvironmentLiveness::Unknown(
        "inspect timed out".to_string(),
    ));

    let error = kill(&registry, &commands, by_ref("C5"))
        .unwrap_err()
        .to_string();

    assert!(
        error.contains("could not be checked (inspect timed out)"),
        "{error}"
    );
    assert_eq!(commands.ran(), ["liveness env-C5"]);
    assert_eq!(
        registry.get("C5").unwrap().status,
        EnvironmentStatus::Stopped
    );
}

#[test]
fn a_stopped_environment_without_a_retained_cleanup_is_refused_before_asking() {
    let registry = EnvironmentRegistry::new();
    registry.commit(EnvironmentRecord {
        retained_cleanup_argv: vec![],
        ..record("C6", EnvironmentStatus::Stopped, EnvironmentOrigin::Created)
    });
    let commands = Commands::answering(EnvironmentLiveness::Gone);

    let error = kill(&registry, &commands, by_ref("C6"))
        .unwrap_err()
        .to_string();

    assert!(error.contains("retained no cleanup"), "{error}");
    assert!(error.contains("quecto container gc"), "{error}");
    assert!(commands.ran().is_empty());
    assert_eq!(
        registry.get("C6").unwrap().status,
        EnvironmentStatus::Stopped
    );
}

#[test]
fn a_failed_removal_stays_owed_and_the_next_kill_retries_the_removal() {
    let registry = EnvironmentRegistry::new();
    registry.commit(record(
        "C7",
        EnvironmentStatus::Stopped,
        EnvironmentOrigin::Created,
    ));
    let commands = Arc::new(Commands {
        liveness: EnvironmentLiveness::Gone,
        cleanup_error: Some("rm: permission denied".to_string()),
        cleanup_failures: Mutex::new(1),
        ran: Mutex::new(Vec::new()),
    });

    let error = kill(&registry, &commands, by_ref("C7")).unwrap_err();

    assert!(
        matches!(&error, KillEnvironmentError::KillFailed { detail, .. } if detail == "rm: permission denied"),
        "{error:?}"
    );
    let record = registry
        .get("C7")
        .expect("never forgotten while its box may remain");
    assert_eq!(record.status, EnvironmentStatus::CleanupFailed);
    assert_eq!(record.last_error.as_deref(), Some("rm: permission denied"));
    assert!(record.removal_pending(), "the removal is owed");
    // The retry is the removal again — liveness, cleanup, forget — never
    // the ordinary kill that would leave it listed stopped.
    let retried = kill(&registry, &commands, by_ref("C7")).expect("retried");
    assert!(retried.removed_stopped);
    assert_eq!(
        commands.ran(),
        [
            "liveness env-C7",
            "cleanup env-C7 kill.sh --op cleanup",
            "liveness env-C7",
            "cleanup env-C7 kill.sh --op cleanup"
        ]
    );
    assert!(registry.get("C7").is_none(), "forgotten");
}

/// #2206 round 3: an owed removal whose container runs again is no longer
/// owed — the box is live — so the kill falls through to the ordinary
/// kill, which ends it and lists it stopped; it never refuses for ever.
#[test]
fn an_owed_removal_whose_container_runs_again_takes_the_ordinary_kill() {
    let registry = EnvironmentRegistry::new();
    let mut owed = record(
        "C9",
        EnvironmentStatus::CleanupFailed,
        EnvironmentOrigin::Created,
    );
    owed.metadata = serde_json::json!({ crate::domain::environments::entities::environment_registry::REMOVAL_PENDING: true });
    registry.commit(owed);
    let commands = Commands::answering(EnvironmentLiveness::Running);

    let killed = kill(&registry, &commands, by_ref("C9")).expect("the ordinary kill");

    assert!(!killed.removed_stopped);
    assert_eq!(commands.ran(), ["liveness env-C9", "kill env-C9"]);
    let record = registry.get("C9").unwrap();
    assert_eq!(record.status, EnvironmentStatus::Stopped);
    assert!(!record.removal_pending(), "no longer owed");
    assert!(record.metadata.get("removal_pending").is_none());
}

/// An owed removal the runtime cannot be asked about stays owed.
#[test]
fn an_owed_removal_whose_liveness_is_unknown_stays_owed() {
    let registry = EnvironmentRegistry::new();
    let mut owed = record(
        "C9",
        EnvironmentStatus::CleanupFailed,
        EnvironmentOrigin::Created,
    );
    owed.metadata = serde_json::json!({ crate::domain::environments::entities::environment_registry::REMOVAL_PENDING: true });
    registry.commit(owed);
    let commands = Commands::answering(EnvironmentLiveness::Unknown("down".into()));

    let error = kill(&registry, &commands, by_ref("C9"))
        .unwrap_err()
        .to_string();

    assert!(error.contains("could not be checked"), "{error}");
    assert_eq!(commands.ran(), ["liveness env-C9"]);
    let record = registry.get("C9").unwrap();
    assert_eq!(record.status, EnvironmentStatus::CleanupFailed);
    assert!(record.removal_pending());
}

#[test]
fn a_live_or_retained_environment_keeps_the_ordinary_kill() {
    // A running box and a retained swarm box are killed exactly as before:
    // no liveness question, no cleanup, the record stays listed stopped.
    for status in [EnvironmentStatus::Running, EnvironmentStatus::Retained] {
        let registry = EnvironmentRegistry::new();
        registry.commit(record("C8", status.clone(), EnvironmentOrigin::Created));
        let commands = Commands::answering(EnvironmentLiveness::Gone);

        let killed = kill(&registry, &commands, by_ref("C8")).expect("killed");

        assert!(!killed.removed_stopped, "{status:?}");
        assert_eq!(commands.ran(), ["kill env-C8"], "{status:?}");
        assert_eq!(
            registry.get("C8").unwrap().status,
            EnvironmentStatus::Stopped
        );
    }
}

/// #2206 round 4: a refused removal returns the record to exactly the
/// status it was claimed from. A stopped record carrying a stale owed mark
/// (a restore set it stopped) stays `stopped` after a refusal, and a second
/// kill of a container still running refuses again — never the ordinary
/// kill of a box just called protected.
#[test]
fn a_refused_removal_of_a_stopped_record_with_a_stale_mark_stays_stopped() {
    for liveness in [
        EnvironmentLiveness::Running,
        EnvironmentLiveness::Unknown("runtime down".into()),
    ] {
        let registry = EnvironmentRegistry::new();
        let mut stale = record(
            "C4",
            EnvironmentStatus::Stopped,
            EnvironmentOrigin::Restored,
        );
        stale.metadata = serde_json::json!({ crate::domain::environments::entities::environment_registry::REMOVAL_PENDING: true });
        registry.commit(stale);
        let commands = Commands::answering(liveness.clone());

        for attempt in 1..=2 {
            let error = kill(&registry, &commands, by_ref("C4"))
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("nothing was removed"),
                "{liveness:?} #{attempt}: {error}"
            );
            let record = registry.get("C4").unwrap();
            assert_eq!(
                record.status,
                EnvironmentStatus::Stopped,
                "{liveness:?} #{attempt}"
            );
            assert!(!record.removal_pending(), "{liveness:?} #{attempt}");
        }
        assert!(
            commands
                .ran()
                .iter()
                .all(|call| call.starts_with("liveness")),
            "{liveness:?}: no kill, no cleanup: {:?}",
            commands.ran()
        );
    }
}

fn swarm(status: RunStatus, outcome: Option<RunStatus>) -> HostedSwarmRun {
    HostedSwarmRun {
        id: "run-7".into(),
        status,
        outcome,
        coordinator: "member-7".into(),
        deadline: 4_102_444_800.0,
    }
}

/// #2206 round 4, the collector's own rule: a stopped box whose checkout
/// hosts a swarm run its owner has not closed — or a board that cannot be
/// read — is the run's, not leftovers. The kill refuses, naming the run,
/// and the record stays stopped.
#[test]
fn a_stopped_box_hosting_an_unfinished_run_is_refused_naming_it() {
    for (hosted, words) in [
        (
            SwarmRunObservation::Run(swarm(RunStatus::Running, None)),
            "hosts unfinished swarm run run-7 (running)",
        ),
        (
            SwarmRunObservation::Run(swarm(RunStatus::Paused, Some(RunStatus::Failed))),
            "hosts unfinished swarm run run-7 (paused holding failed)",
        ),
        (
            SwarmRunObservation::Unreadable("locked".into()),
            "swarm board could not be read (locked)",
        ),
    ] {
        let registry = EnvironmentRegistry::new();
        registry.commit(record(
            "C5",
            EnvironmentStatus::Stopped,
            EnvironmentOrigin::Created,
        ));
        let commands = Commands::answering(EnvironmentLiveness::Gone);

        let error = kill_hosting(&registry, &commands, by_ref("C5"), hosted.clone())
            .unwrap_err()
            .to_string();

        assert!(error.contains(words), "{error}");
        assert_eq!(commands.ran(), ["liveness env-C5"], "no cleanup");
        assert_eq!(
            registry.get("C5").unwrap().status,
            EnvironmentStatus::Stopped
        );
    }
    // A closed run, the placeholder or an unseen workspace: leftovers.
    for hosted in [
        SwarmRunObservation::Run(swarm(RunStatus::Succeeded, None)),
        SwarmRunObservation::Run(HostedSwarmRun {
            deadline: 0.0,
            ..swarm(RunStatus::Setup, None)
        }),
        SwarmRunObservation::NoStoreUnverified,
    ] {
        let registry = EnvironmentRegistry::new();
        registry.commit(record(
            "C6",
            EnvironmentStatus::Stopped,
            EnvironmentOrigin::Created,
        ));
        let commands = Commands::answering(EnvironmentLiveness::Gone);
        let removed = kill_hosting(&registry, &commands, by_ref("C6"), hosted.clone());
        assert!(removed.expect("removed").removed_stopped, "{hosted:?}");
        assert!(registry.get("C6").is_none());
    }
}
