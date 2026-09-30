//! #2247 review F2: a session's restore prints nothing about another
//! session's environment — not even a kill in flight, which it cannot
//! verify — so no `warn` names another session's ref.
use std::sync::{Arc, Mutex};

use super::{build_environment_registry, build_environment_registry_store};
use crate::domain::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentStatus,
};

#[derive(Clone, Default)]
struct CapturedLog(Arc<Mutex<String>>);

impl std::io::Write for CapturedLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap()
            .push_str(&String::from_utf8_lossy(buf));
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedLog {
    type Writer = CapturedLog;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn killing_record_of(created_by: &str) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: "C1".into(),
        environment_id: "env-C1".into(),
        environment_uuid: "uuid-C1".into(),
        name: None,
        workspace_path: "/state/env-C1/workspace".into(),
        repository: String::new(),
        script_name: "default".into(),
        retained_exec_argv: vec!["exec".into()],
        retained_kill_argv: vec!["kill".into()],
        retained_cleanup_argv: vec!["cleanup".into()],
        retained_inspect_argv: vec!["inspect".into()],
        members: vec![],
        status: EnvironmentStatus::Killing,
        metadata: serde_json::json!({}),
        last_error: None,
        origin: EnvironmentOrigin::Created,
        created_by: created_by.into(),
        created_at: Some(1),
    }
}

/// The warnings a session `session` logs restoring a store that holds one
/// killing record created by `created_by`.
fn restore_warnings(created_by: &str, session: &str) -> String {
    let base = tempfile::TempDir::new().unwrap();
    build_environment_registry_store(base.path())
        .record(&killing_record_of(created_by))
        .unwrap();
    let logs = CapturedLog::default();
    let sink = logs.0.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(logs)
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        build_environment_registry(base.path(), session, true);
    });
    sink.lock().unwrap().clone()
}

#[test]
fn another_sessions_kill_in_flight_is_not_warned_about() {
    let warnings = restore_warnings("cli:other", "cli:me");
    assert!(!warnings.contains("C1"), "{warnings}");
}

#[test]
fn a_sessions_own_kill_in_flight_is_warned_about() {
    let warnings = restore_warnings("cli:me", "cli:me");
    assert!(
        warnings.contains("C1") && warnings.contains("could not be verified"),
        "{warnings}"
    );
}

/// The event log, in memory.
#[derive(Default)]
struct RecordedOps(Mutex<Vec<String>>);

impl crate::application::swarm::ports::BoardOpLog for RecordedOps {
    fn record(&self, observation: crate::domain::swarm::BoardOpObservation) {
        self.0.lock().unwrap().push(observation.op);
    }

    /// No summary this test checks is written.
    fn summarize(&self, _summary: crate::domain::swarm::SwarmRunSummary) {}
}

impl crate::application::swarm::ports::SessionOpLog for RecordedOps {
    fn dropped(&self, _drops: crate::application::swarm::dto::DroppedRecords) {}

    fn take_unnoted(&self) -> crate::application::swarm::dto::DroppedRecords {
        crate::application::swarm::dto::DroppedRecords::default()
    }
}

/// #2278 review M1: the host's board calls are recorded in the session's
/// event log. The observation calls the process's board (the one the
/// session's log is bound to) when admission bound one, so a host-side
/// loss record leaves one `_lose_coordinator` swarm_op.
#[test]
fn a_host_side_loss_is_recorded_in_the_process_boards_event_log() {
    use crate::application::environments::ports::HostedSwarmRunObservation;
    use crate::infrastructure::tools::swarm_bridge::{SwarmContext, process_start};
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(workspace.join(".quecto")).unwrap();
    let coordinator = SwarmContext {
        checkout: workspace.clone(),
        member: "coordinator".into(),
        lifecycle: Arc::new(crate::application::swarm::LifecycleService),
        board: super::super::swarm::swarm_board(),
    };
    let identity = crate::domain::swarm::ProcessIdentity {
        pid: std::process::id(),
        started: process_start(std::process::id()).unwrap(),
    };
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 300;
    coordinator
        .create_run(
            &serde_json::json!({"goal": "g", "constraints": [],
                "criteria": [{"id": "t", "kind": "command", "description": "pass"}],
                "member_limit": 3, "deadline": deadline}),
            &identity,
            None,
        )
        .unwrap();
    let record = EnvironmentRecord {
        workspace_path: workspace.clone(),
        metadata: serde_json::json!({"checkout": workspace.display().to_string()}),
        status: EnvironmentStatus::Running,
        ..killing_record_of("cli:me")
    };
    let process_board = super::super::swarm::swarm_board();
    let recorded = Arc::new(RecordedOps::default());
    assert!(process_board.record_in(recorded.clone()));
    let observation = super::hosted_store_observation_over(Some(&process_board));
    let hosted = crate::domain::environment_retention::HostedSwarmRun {
        id: String::new(),
        status: crate::domain::swarm::RunStatus::Running,
        outcome: None,
        coordinator: "coordinator".into(),
        deadline: deadline as f64,
    };
    let loss = futures::executor::block_on(observation.record_lost_coordinator(&record, &hosted))
        .expect("the loss is recorded");
    assert!(loss.lost, "{loss:?}");
    assert_eq!(*recorded.0.lock().unwrap(), ["_lose_coordinator"]);
}
