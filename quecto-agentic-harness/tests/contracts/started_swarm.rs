//! Contract for [`StartedSwarm`] (#2481 S4d): the session's adapter reads a
//! started run's swarm board as the session's live board view
//! (`ReadSwarmBoard`) answers it, from a keeper's own thread: the board's
//! projection, or the view's refusal in its words; and never from one of
//! the runtime's own threads.
use std::path::PathBuf;
use std::sync::Arc;

use quecto::application::environments::dto::{HostedBoardRevision, SwarmBoardSnapshot};
use quecto::application::environments::ports::{HostedSwarmBoard, PortFuture};
use quecto::application::environments::use_cases::ReadSwarmBoard;
use quecto::application::project_board::ports::StartedSwarm;
use quecto::domain::environments::entities::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};
use quecto::interface::tools::project_board::keeper::SessionSwarmBoards;
use serde_json::json;

/// Environment `C1` hosts a board; any other hosts none.
struct OneBoard;

impl HostedSwarmBoard for OneBoard {
    fn board<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, Result<Option<SwarmBoardSnapshot>, String>> {
        let found = (record.environment_ref == "C1")
            .then(|| SwarmBoardSnapshot::object(json!({"revision": 4, "run": null})))
            .flatten();
        Box::pin(async move { Ok(found) })
    }

    fn revision<'a>(
        &'a self,
        _record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, Result<Option<HostedBoardRevision>, String>> {
        Box::pin(async move { Ok(None) })
    }
}

fn record(reference: &str) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: reference.into(),
        environment_id: format!("runtime-{reference}"),
        environment_uuid: format!("uuid-{reference}"),
        name: None,
        workspace_path: PathBuf::from(format!("/workspace/{reference}")),
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

#[test]
fn a_started_run_s_swarm_is_read_as_the_live_board_view_reads_it() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let registry = EnvironmentRegistry::new();
    registry.commit(record("C1"));
    registry.commit(record("C2"));
    let read = Arc::new(ReadSwarmBoard::new(registry, Arc::new(OneBoard)));
    let boards = SessionSwarmBoards::new(
        Arc::new(move || Some(Arc::clone(&read))),
        runtime.handle().clone(),
    );
    assert_eq!(boards.board("C1"), Ok(json!({"revision": 4, "run": null})));
    assert_eq!(
        boards.board("C2"),
        Err("swarm C2 hosts no board this session can read".into())
    );
    assert_eq!(boards.board("C9"), Err("unknown swarm C9".into()));
    // Read from one of the runtime's own threads, it refuses.
    let on_runtime = runtime.block_on(async { boards.board("C1") });
    assert_eq!(
        on_runtime,
        Err("a swarm board is read off the async runtime".into())
    );
}

#[test]
fn a_session_with_no_swarm_boards_yet_reads_none() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let boards = SessionSwarmBoards::new(Arc::new(|| None), runtime.handle().clone());
    assert_eq!(
        boards.board("C1"),
        Err("swarm boards are unavailable in this session".into())
    );
}
