use super::ReadSwarmBoard;
use crate::application::environments::dto::{
    BoardSighting, HostedBoardRevision, SwarmBoardRefusal,
};
use crate::application::environments::ports::{HostedSwarmBoard, PortFuture};
use crate::domain::environments::entities::environment_registry::{
    EnvironmentJournal, EnvironmentOrigin, EnvironmentRecord, EnvironmentRegistry,
    EnvironmentStatus, JournalWrite,
};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// An environment this session created.
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

/// An environment another session created, restored from the durable
/// registry every session shares.
fn foreign(reference: &str, status: EnvironmentStatus) -> EnvironmentRecord {
    EnvironmentRecord {
        origin: EnvironmentOrigin::Restored,
        created_by: "another-session".into(),
        ..record(reference, status)
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
    ) -> PortFuture<'a, Result<Option<Map<String, Value>>, String>> {
        let found = self.find(record).map(|board| {
            board.map(|revision| Map::from_iter([("revision".to_owned(), json!(revision))]))
        });
        Box::pin(async move { found })
    }

    fn revision<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, Result<Option<HostedBoardRevision>, String>> {
        let run = format!("run-{}", record.environment_ref);
        let found = self.find(record).map(|board| {
            board.map(|revision| HostedBoardRevision {
                run: Some(run),
                revision,
            })
        });
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
    assert_eq!(
        read.board("C1").await,
        Ok(Map::from_iter([("revision".to_owned(), json!(7))]))
    );
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

/// L6 (#2488 review): only an environment ref (`C` and digits) is looked
/// up, and what was sent in its place is never echoed.
#[tokio::test]
async fn a_swarm_must_be_an_environment_ref() {
    let read = read(
        Boards(vec![("C1", Ok(7))]),
        vec![record("C1", EnvironmentStatus::Running)],
    );
    let long = format!("C1{}", "x".repeat(10_000));
    for sent in [
        "",
        "c1",
        "C",
        "C1x",
        " C1",
        "C99999999999999999999999",
        &long,
    ] {
        let refused = read.board(sent).await.unwrap_err();
        assert_eq!(refused, SwarmBoardRefusal::NotARef, "{sent:?}");
        assert_eq!(
            refused.to_string(),
            "swarm must be an environment ref such as C1"
        );
    }
}

/// M2 and L6 (#2488 review): the view reads this session's own swarms
/// (as `get_containers` lists without `all`), and only those that may
/// still hold a board (running or retained), as the poll does.
#[tokio::test]
async fn only_this_session_s_running_or_retained_swarms_are_read() {
    let boards = Boards(vec![
        ("C4", Ok(1)),
        ("C5", Ok(2)),
        ("C6", Ok(3)),
        ("C7", Ok(4)),
    ]);
    let records = vec![
        foreign("C4", EnvironmentStatus::Running),
        record("C5", EnvironmentStatus::Stopped),
        record("C6", EnvironmentStatus::Killing),
        record("C7", EnvironmentStatus::Retained),
    ];
    let read = read(boards, records);
    assert_eq!(
        read.board("C4").await,
        Err(SwarmBoardRefusal::OtherSession("C4".into()))
    );
    for stopped in ["C5", "C6"] {
        assert_eq!(
            read.board(stopped).await,
            Err(SwarmBoardRefusal::NotLive(stopped.into()))
        );
    }
    assert!(read.board("C7").await.is_ok(), "a retained board is read");
}

/// L6 (#2488 review): a registry that could not be read answers with its
/// own account, never "unknown swarm".
#[tokio::test]
async fn a_registry_that_cannot_be_read_says_so() {
    let journal = EnvironmentJournal {
        allocate_ref: Arc::new(|_| Ok(1)),
        release_ref: Arc::new(|_| {}),
        recorded: Arc::new(|_, _| JournalWrite::Written),
        forgotten: Arc::new(|_| {}),
        reload: Arc::new(|| Err("environments.json: corrupt".into())),
    };
    let registry =
        EnvironmentRegistry::unreadable(journal, "session", "environments.json: corrupt");
    let read = ReadSwarmBoard::new(registry, Arc::new(Boards(vec![])));
    assert_eq!(
        read.board("C1").await,
        Err(SwarmBoardRefusal::Unreadable(
            "registry unreadable: environments.json: corrupt".into()
        ))
    );
}

/// The poll's sightings: this session's running and retained boards, by
/// ref, each with the run it counts; one that cannot be read is said to
/// be unreadable (the poll keeps what it last saw); stopped, killing,
/// boardless and other sessions' environments are left out.
#[tokio::test]
async fn sightings_cover_this_session_s_running_and_retained_boards() {
    let boards = Boards(vec![
        ("C1", Ok(3)),
        ("C2", Ok(9)),
        ("C3", Err("contended")),
        ("C4", Ok(1)),
        ("C5", Ok(2)),
        ("C7", Ok(5)),
    ]);
    let records = vec![
        record("C1", EnvironmentStatus::Running),
        record("C2", EnvironmentStatus::Retained),
        record("C3", EnvironmentStatus::Running),
        record("C4", EnvironmentStatus::Stopped),
        record("C5", EnvironmentStatus::Killing),
        record("C6", EnvironmentStatus::Running),
        foreign("C7", EnvironmentStatus::Running),
    ];
    let board = |swarm: &str, revision| {
        BoardSighting::Board(HostedBoardRevision {
            run: Some(format!("run-{swarm}")),
            revision,
        })
    };
    assert_eq!(
        read(boards, records).sightings().await,
        BTreeMap::from([
            ("C1".to_owned(), board("C1", 3)),
            ("C2".to_owned(), board("C2", 9)),
            (
                "C3".to_owned(),
                BoardSighting::Unreadable("contended".into())
            ),
        ])
    );
}
