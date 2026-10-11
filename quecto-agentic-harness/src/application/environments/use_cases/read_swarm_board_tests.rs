use super::ReadSwarmBoard;
use crate::application::environments::dto::SwarmBoardRefusal;
use crate::application::environments::ports::{HostedSwarmBoard, PortFuture};
use crate::domain::environments::entities::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

fn record(reference: &str, status: EnvironmentStatus) -> EnvironmentRecord {
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
        status,
        metadata: json!({}),
        last_error: None,
        origin: EnvironmentOrigin::Created,
        created_by: String::new(),
        created_at: None,
    }
}

/// Boards by environment ref: a revision, or an error text; absent means
/// the environment hosts no board.
struct Boards(Vec<(&'static str, Result<i64, &'static str>)>);

impl Boards {
    fn find(&self, record: &EnvironmentRecord) -> Result<Option<i64>, String> {
        let found = self
            .0
            .iter()
            .find(|(name, _)| *name == record.environment_ref);
        found
            .map(|(_, board)| board.map_err(str::to_owned))
            .transpose()
    }
}

impl HostedSwarmBoard for Boards {
    fn board<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, Result<Option<Value>, String>> {
        let found = self
            .find(record)
            .map(|board| board.map(|revision| json!({"revision": revision})));
        Box::pin(async move { found })
    }

    fn revision<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, Result<Option<i64>, String>> {
        let found = self.find(record);
        Box::pin(async move { found })
    }
}

fn read(boards: Boards, records: Vec<EnvironmentRecord>) -> ReadSwarmBoard {
    let registry = EnvironmentRegistry::new();
    records
        .into_iter()
        .for_each(|record| registry.commit(record));
    ReadSwarmBoard::new(registry, Arc::new(boards))
}

#[tokio::test]
async fn a_swarm_is_read_by_its_environment_ref() {
    let boards = Boards(vec![("C1", Ok(7)), ("C2", Err("contended"))]);
    let records = vec![
        record("C1", EnvironmentStatus::Running),
        record("C2", EnvironmentStatus::Running),
        record("C3", EnvironmentStatus::Running),
    ];
    let read = read(boards, records);
    assert_eq!(read.board("C1").await, Ok(json!({"revision": 7})));
    assert_eq!(
        read.board("C2").await,
        Err(SwarmBoardRefusal::Unreadable("contended".into()))
    );
    assert_eq!(
        read.board("C3").await,
        Err(SwarmBoardRefusal::NoBoard("C3".into()))
    );
    assert_eq!(
        read.board("C9").await,
        Err(SwarmBoardRefusal::UnknownSwarm("C9".into()))
    );
}

#[tokio::test]
async fn revisions_cover_running_and_retained_boards_that_read() {
    let boards = Boards(vec![
        ("C1", Ok(3)),
        ("C2", Ok(9)),
        ("C3", Err("contended")),
        ("C4", Ok(1)),
        ("C5", Ok(2)),
    ]);
    let records = vec![
        record("C1", EnvironmentStatus::Running),
        record("C2", EnvironmentStatus::Retained),
        record("C3", EnvironmentStatus::Running),
        record("C4", EnvironmentStatus::Stopped),
        record("C5", EnvironmentStatus::Killing),
        record("C6", EnvironmentStatus::Running),
    ];
    let mut revisions = read(boards, records).revisions().await;
    revisions.sort();
    assert_eq!(revisions, [("C1".to_owned(), 3), ("C2".to_owned(), 9)]);
}
