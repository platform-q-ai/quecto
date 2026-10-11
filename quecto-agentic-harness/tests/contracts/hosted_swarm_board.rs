//! Contract for [`HostedSwarmBoard`] (#2478), proven on the production
//! store adapter: an environment hosting no board reads as none, and a
//! created run's board answers the `get_swarm_board` snapshot's shape at
//! the board's revision, which a write moves.
use std::sync::Arc;

use quecto::application::environments::ports::HostedSwarmBoard;
use quecto::domain::environments::entities::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentStatus, mint_environment_uuid,
};
use quecto::infrastructure::tools::environment_commands::HostedStoreObservation;
use quecto::infrastructure::tools::swarm_bridge::SwarmContext;
use serde_json::json;

fn port() -> Arc<dyn HostedSwarmBoard> {
    Arc::new(HostedStoreObservation::new(
        quecto::composition::swarm::swarm_board(),
    ))
}

fn record(checkout: &std::path::Path) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: "C1".into(),
        environment_id: "env-contract".into(),
        environment_uuid: mint_environment_uuid(),
        name: None,
        workspace_path: checkout.to_path_buf(),
        repository: String::new(),
        script_name: "default".into(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec!["true".into()],
        retained_cleanup_argv: vec![],
        retained_inspect_argv: vec![],
        members: vec![],
        status: EnvironmentStatus::Running,
        metadata: json!({ "checkout": checkout.display().to_string() }),
        last_error: None,
        origin: EnvironmentOrigin::Created,
        created_by: String::new(),
        created_at: None,
    }
}

fn store(checkout: &std::path::Path) -> SwarmContext {
    std::fs::create_dir_all(checkout.join(".quecto")).unwrap();
    SwarmContext {
        board: quecto::composition::swarm::swarm_board(),
        checkout: checkout.to_path_buf(),
        member: "coordinator".into(),
        lifecycle: Arc::new(quecto::application::swarm::LifecycleService),
    }
}

fn create_run(context: &SwarmContext) {
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 600;
    quecto::infrastructure::tools::call_work::off_the_runtime(|| {
        context.create_run(
            &json!({"goal":"ship", "constraints":[], "criteria":[{"id":"tests","kind":"command","description":"pass"}], "member_limit":3, "deadline":deadline}),
            &quecto::domain::swarm::ProcessIdentity {
                pid: std::process::id(),
                started: quecto::infrastructure::tools::swarm_bridge::process_start(std::process::id())
                    .unwrap(),
            },
            None,
        )
    })
    .unwrap();
}

#[tokio::test]
async fn an_environment_without_a_board_reads_as_none() {
    let temp = tempfile::tempdir().unwrap();
    let record = record(temp.path());
    assert_eq!(port().board(&record).await, Ok(None));
    assert_eq!(port().revision(&record).await, Ok(None));
}

#[tokio::test]
async fn a_hosted_board_answers_the_snapshot_shape_at_its_revision() {
    let temp = tempfile::tempdir().unwrap();
    let context = store(temp.path());
    create_run(&context);
    let record = record(temp.path());
    let board = port().board(&record).await.unwrap().expect("a board");
    let keys: Vec<&str> = board
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "revision",
            "run",
            "items",
            "members",
            "evidence",
            "totals",
            "truncated"
        ]
    );
    let run: Vec<&str> = board["run"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        run,
        [
            "id",
            "goal",
            "status",
            "outcome",
            "outcome_reason",
            "generation",
            "criteria",
            "member_limit",
            "deadline",
            "coordinator",
            "integrator",
            "budget",
        ]
    );
    assert_eq!(
        (
            board["run"]["goal"].clone(),
            board["run"]["member_limit"].clone()
        ),
        (json!("ship"), json!(3))
    );
    assert_eq!(board["members"][0]["role"], json!("coordinator"));
    let revision = port().revision(&record).await.unwrap().expect("a revision");
    assert_eq!(board["revision"], json!(revision));
    let connection = rusqlite::Connection::open(temp.path().join(".quecto/swarm.sqlite")).unwrap();
    connection
        .execute_batch(
            "INSERT INTO tasks (title, acceptance, dependencies, status, evidence) VALUES ('Plumbing', '[\"passes\"]', '[]', 'ready', '[]');
             INSERT INTO events (actor, time, action, detail) VALUES ('coordinator', 1.0, 'task_created', '{}');",
        )
        .unwrap();
    let after = port().board(&record).await.unwrap().unwrap();
    assert!(
        port().revision(&record).await.unwrap().unwrap() > revision,
        "a write moves it"
    );
    let item: Vec<&str> = after["items"][0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        item,
        [
            "id",
            "title",
            "criteria",
            "depends_on",
            "state",
            "waiting",
            "owner",
            "blocker",
            "reserved_paths",
            "evidence"
        ]
    );
}
