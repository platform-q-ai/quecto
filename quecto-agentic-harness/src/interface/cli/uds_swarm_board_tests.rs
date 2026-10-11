use super::{answer, changes};
use crate::application::environments::ports::{HostedSwarmBoard, PortFuture};
use crate::application::environments::use_cases::ReadSwarmBoard;
use crate::domain::environments::entities::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

struct NoMembers;

impl crate::application::environments::ports::EnvironmentMemberShutdown for NoMembers {
    fn shutdown_members<'a>(
        &'a self,
        _members: &'a [String],
    ) -> PortFuture<'a, crate::application::environments::ports::MemberShutdownReport> {
        Box::pin(async {
            crate::application::environments::ports::MemberShutdownReport {
                settled: vec![],
                unsettled: vec![],
            }
        })
    }
}

/// A board at `C1` only.
struct OneBoard;

impl HostedSwarmBoard for OneBoard {
    fn board<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, Result<Option<Value>, String>> {
        let board = (record.environment_ref == "C1")
            .then(|| json!({"revision": 7, "run": null, "items": [], "truncated": false}));
        Box::pin(async move { Ok(board) })
    }

    fn revision<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, Result<Option<i64>, String>> {
        let revision = (record.environment_ref == "C1").then_some(7);
        Box::pin(async move { Ok(revision) })
    }
}

fn record(reference: &str) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: reference.into(),
        environment_id: format!("runtime-{reference}"),
        environment_uuid: format!("uuid-{reference}"),
        name: None,
        workspace_path: "/workspace".into(),
        repository: String::new(),
        script_name: "default".into(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec![],
        retained_cleanup_argv: vec![],
        retained_inspect_argv: vec![],
        members: vec![],
        status: EnvironmentStatus::Running,
        metadata: json!({}),
        last_error: None,
        origin: EnvironmentOrigin::Created,
        created_by: String::new(),
        created_at: None,
    }
}

fn boards() -> ReadSwarmBoard {
    let registry = EnvironmentRegistry::new();
    registry.commit(record("C1"));
    registry.commit(record("C2"));
    ReadSwarmBoard::new(registry, Arc::new(OneBoard))
}

fn wire(event: crate::interface::cli::protocol::AgentEvent) -> Value {
    serde_json::from_str(&event.to_json_line()).unwrap()
}

#[tokio::test]
async fn get_swarm_board_answers_the_board_with_its_swarm() {
    let response = wire(answer(Some(&boards()), Some("b1"), Some("C1")).await);
    assert_eq!(
        (response["type"].clone(), response["command"].clone()),
        (json!("response"), json!("get_swarm_board"))
    );
    assert_eq!(
        (response["id"].clone(), response["success"].clone()),
        (json!("b1"), json!(true))
    );
    let data = &response["data"];
    assert_eq!(
        (
            data["swarm"].clone(),
            data["revision"].clone(),
            data["truncated"].clone()
        ),
        (json!("C1"), json!(7), json!(false))
    );
}

#[tokio::test]
async fn get_swarm_board_refuses_an_unknown_boardless_or_missing_swarm() {
    let boards = boards();
    for (swarm, error) in [
        (Some("C9"), "unknown swarm C9"),
        (Some("C2"), "swarm C2 hosts no board this session can read"),
        (None, "swarm is required (an environment ref such as C1)"),
    ] {
        let response = wire(answer(Some(&boards), Some("b1"), swarm).await);
        assert_eq!(
            (response["success"].clone(), response["error"].clone()),
            (json!(false), json!(error)),
            "{swarm:?}"
        );
    }
    let unavailable = wire(answer(None, None, Some("C1")).await);
    assert_eq!(
        unavailable["error"],
        json!("swarm boards are unavailable in this harness")
    );
}

#[test]
fn a_swarm_is_hinted_when_first_seen_and_when_its_revision_moves() {
    let mut seen = BTreeMap::new();
    let hint = |swarm: &str, revision: i64| {
        json!({"type": "swarm_board_changed", "swarm": swarm, "revision": revision}).to_string()
    };
    let first = changes(&mut seen, vec![("C1".into(), 3), ("C2".into(), 9)]);
    assert_eq!(first, [hint("C1", 3), hint("C2", 9)]);
    assert_eq!(
        changes(&mut seen, vec![("C1".into(), 3), ("C2".into(), 9)]),
        Vec::<String>::new(),
        "unchanged: no hint"
    );
    assert_eq!(
        changes(&mut seen, vec![("C1".into(), 4), ("C2".into(), 9)]),
        [hint("C1", 4)]
    );
    assert_eq!(
        changes(&mut seen, vec![("C1".into(), 4)]),
        Vec::<String>::new(),
        "a board no longer read"
    );
    assert_eq!(
        changes(&mut seen, vec![("C1".into(), 4), ("C2".into(), 9)]),
        [hint("C2", 9)],
        "is hinted again when it reads"
    );
}

#[tokio::test(start_paused = true)]
async fn hints_are_polled_at_most_once_a_second_and_only_with_a_client() {
    use std::sync::atomic::{AtomicU32, Ordering};
    let slot =
        crate::infrastructure::tools::agent_cmd_containers::EnvironmentControlSlot::default();
    let registry = EnvironmentRegistry::new();
    registry.commit(record("C1"));
    let control = crate::infrastructure::tools::agent_cmd_containers::EnvironmentControl {
        list: Arc::new(crate::application::environments::use_cases::ListEnvironmentsQuery::new(registry.clone())),
        kill: Arc::new(crate::application::environments::use_cases::KillEnvironment::new(
            registry.clone(),
            Arc::new(NoMembers),
            Arc::new(crate::infrastructure::tools::environment_commands::ScriptEnvironmentCommands::default()),
            Arc::new(crate::infrastructure::tools::environment_commands::HostedStoreObservation::new(
                crate::composition::swarm::swarm_board(),
            )),
        )),
        boards: Arc::new(ReadSwarmBoard::new(registry, Arc::new(OneBoard))),
    };
    assert!(slot.install(control));
    let (tx, mut rx) = tokio::sync::broadcast::channel(16);
    let clients = Arc::new(AtomicU32::new(0));
    let task = super::spawn_change_hints(slot, tx, clients.clone());
    tokio::time::sleep(std::time::Duration::from_millis(2_500)).await;
    assert!(rx.try_recv().is_err(), "no client, no poll");
    clients.store(1, Ordering::SeqCst);
    tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
    assert_eq!(
        rx.try_recv().unwrap(),
        json!({"type": "swarm_board_changed", "swarm": "C1", "revision": 7}).to_string()
    );
    tokio::time::sleep(std::time::Duration::from_millis(3_000)).await;
    assert!(
        rx.try_recv().is_err(),
        "an unmoved board is not hinted again"
    );
    task.abort();
}
