//! Contract for [`SnapshotFiles`] (epic #2705 B4), proven on the markdown
//! adapter: a snapshot's files are one per artifact, in path order, each
//! at the path a read of the board spells for its key, and each reads
//! back, by its kind's codec, as the very artifact it was made of.
use quecto::application::project_board::ports::{BoardHead, BoardSnapshot, SnapshotFiles};
use quecto::domain::project_board::entities::task::{Task, TaskFields};
use quecto::domain::project_board::value_objects::slug::Slug;
use quecto::domain::project_board::value_objects::timestamp::Timestamp;
use quecto::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};
use quecto::infrastructure::persistence::project_board::artifacts::{
    change::ChangeCodec, decode, discussion::DiscussionCodec, plan::PlanCodec, review::ReviewCodec,
    run::RunCodec, task::TaskCodec,
};
use quecto::infrastructure::persistence::project_board::snapshot_files::MarkdownSnapshotFiles;
use quecto::infrastructure::test_support::board_artifacts::{
    change, discussion, plan, review, run,
};

fn task(id: &str) -> Task {
    let at = Timestamp::parse("2026-10-24T08:00:00Z").unwrap();
    Task::new(TaskFields {
        id: Slug::parse(id).unwrap(),
        title: format!("Task {id}"),
        kind: TaskKind::Task,
        description: "Do it.".into(),
        status: TaskStatus::Ready,
        parent: None,
        depends_on: vec![],
        accepted_plan: None,
        claim: None,
        prs: vec![],
        github_issue: None,
        resolution: None,
        superseded_by: vec![],
        references: vec![],
        created: at.clone(),
        updated: at,
    })
    .unwrap()
}

/// A board of one artifact of each kind, and a second task.
fn snapshot() -> BoardSnapshot {
    BoardSnapshot {
        head: BoardHead("0".repeat(40)),
        tasks: vec![task("zeta"), task("alpha")],
        plans: vec![plan("alpha", 2)],
        runs: vec![run("alpha", 1)],
        changes: vec![change("alpha", 3)],
        reviews: vec![review("alpha", 3)],
        discussions: vec![discussion("why")],
        quarantined: vec![],
    }
}

#[test]
fn each_artifact_is_one_file_at_its_path_in_path_order() {
    let files = MarkdownSnapshotFiles.files(&snapshot()).unwrap();
    let paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "changes/alpha.3.md",
            "discussions/why.md",
            "plans/alpha.2.md",
            "reviews/alpha.3.md",
            "runs/alpha.1.md",
            "tasks/alpha.md",
            "tasks/zeta.md",
        ]
    );
}

#[test]
fn each_file_reads_back_as_its_artifact() {
    let board = snapshot();
    let files = MarkdownSnapshotFiles.files(&board).unwrap();
    let bytes = |path: &str| {
        files
            .iter()
            .find(|file| file.path == path)
            .map(|file| file.bytes.clone())
            .unwrap_or_else(|| panic!("no file {path}"))
    };
    assert_eq!(
        decode::<TaskCodec>(&bytes("tasks/alpha.md")).unwrap(),
        board.tasks[1]
    );
    assert_eq!(
        decode::<TaskCodec>(&bytes("tasks/zeta.md")).unwrap(),
        board.tasks[0]
    );
    assert_eq!(
        decode::<PlanCodec>(&bytes("plans/alpha.2.md")).unwrap(),
        board.plans[0]
    );
    assert_eq!(
        decode::<RunCodec>(&bytes("runs/alpha.1.md")).unwrap(),
        board.runs[0]
    );
    assert_eq!(
        decode::<ChangeCodec>(&bytes("changes/alpha.3.md")).unwrap(),
        board.changes[0]
    );
    assert_eq!(
        decode::<ReviewCodec>(&bytes("reviews/alpha.3.md")).unwrap(),
        board.reviews[0]
    );
    assert_eq!(
        decode::<DiscussionCodec>(&bytes("discussions/why.md")).unwrap(),
        board.discussions[0]
    );
}

#[test]
fn an_empty_board_is_no_files() {
    let empty = BoardSnapshot {
        head: BoardHead("0".repeat(40)),
        tasks: vec![],
        plans: vec![],
        runs: vec![],
        changes: vec![],
        reviews: vec![],
        discussions: vec![],
        quarantined: vec![],
    };
    assert!(MarkdownSnapshotFiles.files(&empty).unwrap().is_empty());
}
