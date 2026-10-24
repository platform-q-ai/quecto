//! Contract for [`SnapshotFiles`] (epic #2705 B4), proven on the git
//! adapter: a snapshot's files are the bytes the store keeps, byte for
//! byte, never a re-encoding. One per artifact read, at the path a read
//! spells for its key; one per file set aside, with its reason; in path
//! order; and a snapshot past the bound is refused as too large.
use super::project_board_git_store::{Remote, sh};
use quecto::application::project_board::ports::{
    BoardArtifact, BoardWrite, ProjectBoardStore, SnapshotFiles, SnapshotFilesError,
};
use quecto::domain::project_board::entities::task::{Task, TaskFields};
use quecto::domain::project_board::value_objects::slug::Slug;
use quecto::domain::project_board::value_objects::timestamp::Timestamp;
use quecto::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};
use quecto::infrastructure::persistence::project_board::artifacts::file_of;
use quecto::infrastructure::persistence::project_board::board_tree::FILTER_BYTES;
use quecto::infrastructure::persistence::project_board::snapshot_files::GitSnapshotFiles;
use quecto::infrastructure::test_support::board_artifacts::{
    change, discussion, plan, review, run,
};

/// A row the plan codec reads past and never writes (#2481).
const LEGACY_ROW: &str = "| Budget tokens | 5000 |\n";

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

/// A board of one artifact of each kind, whose plan a hand then gave a
/// legacy row the codec reads past, and beside which it put a stray task
/// file no codec reads. Gives the legacy plan's bytes.
fn board(remote: &Remote) -> Vec<u8> {
    let store = remote.client(0, "Alice");
    store.bootstrap().unwrap();
    let head = store.read().unwrap().head;
    let artifacts = [
        BoardArtifact::Task(task("a")),
        BoardArtifact::Plan(plan("a", 1)),
        BoardArtifact::Run(run("a", 1)),
        BoardArtifact::Change(change("a", 3)),
        BoardArtifact::Review(review("a", 3)),
        BoardArtifact::Discussion(discussion("why")),
    ];
    let written = store.write(&head, &artifacts, "seed").unwrap();
    assert!(matches!(written, BoardWrite::Written(_)), "{written:?}");
    let plan = String::from_utf8(file_of(&artifacts[1]).unwrap()).unwrap();
    let at = plan
        .find("| Created |")
        .expect("a plan has its Created row");
    let legacy = format!("{}{LEGACY_ROW}{}", &plan[..at], &plan[at..]);
    remote.put("plans", "a.1.md", legacy.as_bytes());
    remote.put("tasks", "broken.md", b"not a task\n");
    legacy.into_bytes()
}

#[test]
fn the_files_are_the_stored_bytes_set_aside_included() {
    let remote = Remote::new();
    let legacy = board(&remote);
    let snapshot = remote.client(0, "Alice").read().unwrap();
    assert_eq!(snapshot.plans.len(), 1, "the legacy plan reads");
    assert_eq!(snapshot.quarantined.len(), 1, "the stray is set aside");
    let files = GitSnapshotFiles::new(remote.git(0, "Alice"))
        .files(&snapshot)
        .unwrap();
    let paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "changes/a.3.md",
            "discussions/why.md",
            "plans/a.1.md",
            "reviews/a.3.md",
            "runs/a.1.md",
            "tasks/a.md",
            "tasks/broken.md",
        ]
    );
    for file in &files {
        let bytes = file.bytes.as_deref().expect("every file here is a blob");
        let object = sh(remote.clone_dir(0), &["hash-object", "--stdin"], bytes);
        assert_eq!(object, remote.object(&file.path), "{} as stored", file.path);
        let set_aside = file.path == "tasks/broken.md";
        assert_eq!(file.set_aside.is_some(), set_aside, "{}", file.path);
    }
    // The legacy plan as kept: never re-encoded without its row.
    let plan = files
        .iter()
        .find(|file| file.path == "plans/a.1.md")
        .unwrap();
    assert_eq!(plan.bytes.as_deref(), Some(legacy.as_slice()));
}

#[test]
fn a_snapshot_past_the_bound_is_refused_as_too_large() {
    let remote = Remote::new();
    board(&remote);
    let snapshot = remote.client(0, "Alice").read().unwrap();
    let refused = GitSnapshotFiles::new(remote.git(0, "Alice"))
        .bounded(10)
        .files(&snapshot);
    assert!(
        matches!(refused, Err(SnapshotFilesError::TooLarge(_))),
        "{refused:?}"
    );
}

/// Review round 2: the bound is the sum over the paths served, so a blob
/// three paths share counts three times.
#[test]
fn the_bound_counts_a_shared_blob_once_per_path() {
    let remote = Remote::new();
    board(&remote);
    remote.put("tasks", "copy-1.md", b"not a task\n");
    remote.put("tasks", "copy-2.md", b"not a task\n");
    let snapshot = remote.client(0, "Alice").read().unwrap();
    let files = GitSnapshotFiles::new(remote.git(0, "Alice"));
    let served = files.clone().files(&snapshot).unwrap();
    let copies = served
        .iter()
        .filter(|file| file.bytes.as_deref() == Some(&b"not a task\n"[..]))
        .count();
    assert_eq!(copies, 3, "broken.md and its two copies");
    let total: usize = served
        .iter()
        .map(|file| file.bytes.as_ref().map_or(0, Vec::len))
        .sum();
    assert!(files.clone().bounded(total).files(&snapshot).is_ok());
    let refused = files.bounded(total - 1).files(&snapshot);
    assert!(
        matches!(refused, Err(SnapshotFilesError::TooLarge(_))),
        "{refused:?}"
    );
}

/// Review round 2: a set-aside blob past every read cap is served with no
/// bytes and why, as a partial clone (which never fetches it) serves it.
#[test]
fn a_set_aside_blob_past_every_cap_is_served_without_bytes() {
    let remote = Remote::new();
    board(&remote);
    let huge = vec![b'x'; FILTER_BYTES];
    remote.put("plans", "huge.bin", &huge);
    let snapshot = remote.client(0, "Alice").read().unwrap();
    let served = GitSnapshotFiles::new(remote.git(0, "Alice"))
        .files(&snapshot)
        .unwrap();
    let file = served.iter().find(|file| file.path == "plans/huge.bin");
    let file = file.unwrap_or_else(|| panic!("served: {served:?}"));
    assert_eq!(file.bytes, None);
    let why = file.set_aside.as_deref().unwrap_or_default();
    assert!(
        why.ends_with(&format!("too large to serve: {FILTER_BYTES} bytes")),
        "{why}"
    );
}

#[test]
fn a_snapshot_whose_head_the_repository_lacks_is_unreadable() {
    let remote = Remote::new();
    board(&remote);
    let mut snapshot = remote.client(0, "Alice").read().unwrap();
    snapshot.head.0 = "0".repeat(40);
    let refused = GitSnapshotFiles::new(remote.git(0, "Alice")).files(&snapshot);
    assert!(
        matches!(refused, Err(SnapshotFilesError::Unreadable(_))),
        "{refused:?}"
    );
}
