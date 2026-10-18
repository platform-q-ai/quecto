//! The board keeps each task as its canonical markdown, `tasks/<id>.md`
//! (#2481 S8): the task codec's file, read only when it decodes to the task
//! its name spells. Anything else under a task's name is set aside under
//! that id and carried forward as it is, a task file of the JSON form older
//! quecto wrote included, so its id stays blocked until it is cleared.
use super::*;
use crate::application::project_board::ports::{ArtifactKey, QuarantinedFile, SetAsideClear};
use crate::infrastructure::persistence::project_board::artifacts::task::TaskCodec;
use crate::infrastructure::persistence::project_board::artifacts::{self, ArtifactCodec};

/// The canonical file of `task`.
fn canonical(task: &Task) -> String {
    artifacts::encode::<TaskCodec>(task).unwrap()
}

/// `bytes` as a blob of the remote, as a tree entry.
fn blob(remote: &Path, bytes: &[u8]) -> String {
    format!(
        "100644 blob {}",
        sh_with(remote, &["hash-object", "-w", "--stdin"], bytes)
    )
}

/// A board holding task `b`, then `extra` beside it by hand: the store's
/// read and the object each extra file has.
fn hand_edited(extra: &[(&str, Vec<u8>)]) -> (Repos, BoardSnapshot) {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    let head = created(&alice);
    written(alice.write_tasks(&head, &[task("b")], "board: add b"));
    let remote = repos.remote();
    let entries: Vec<(&str, String)> = extra
        .iter()
        .map(|(name, bytes)| (*name, blob(&remote, bytes)))
        .collect();
    hand_edit(&remote, &entries);
    let snapshot = store(&repos.bob).read().unwrap();
    (repos, snapshot)
}

/// The one file set aside at `path`, and whether a write carries it.
fn aside<'a>(snapshot: &'a BoardSnapshot, path: &str) -> (&'a QuarantinedFile, bool) {
    let files: Vec<&QuarantinedFile> = snapshot
        .quarantined
        .iter()
        .filter(|file| file.path == path)
        .collect();
    assert_eq!(files.len(), 1, "{:?}", snapshot.quarantined);
    (files[0], files[0].carried)
}

/// A write of another task carries the file at `path` as it is, and a
/// write of the task it blocks is refused.
fn carried_and_blocking(repos: &Repos, snapshot: &BoardSnapshot, path: &str, blocked: &str) {
    let bob = store(&repos.bob);
    let (file, _) = aside(snapshot, path);
    let object = file.object.clone();
    let head = written(bob.write_tasks(&snapshot.head, &[task("c")], "board: add c"));
    let name = path.strip_prefix(TASKS_PATH).unwrap();
    assert_eq!(
        sh(
            &repos.remote(),
            &["rev-parse", &format!("{}:tasks/{name}", head.0)]
        ),
        object
    );
    let refused = bob.write_tasks(&head, &[task(blocked)], "board: add it");
    assert!(
        matches!(&refused, Ok(BoardWrite::Refused(why)) if why.contains(path)),
        "{refused:?}"
    );
}

const TASKS_PATH: &str = "tasks/";

#[test]
fn a_task_is_stored_as_its_canonical_markdown() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    let head = created(&alice);
    let head = written(alice.write_tasks(&head, &[task("a")], "board: add a"));
    let remote = repos.remote();
    assert_eq!(
        sh(
            &remote,
            &["ls-tree", "--name-only", &format!("{}:tasks", head.0)]
        ),
        "a.md"
    );
    let file = sh(
        &remote,
        &[
            "cat-file",
            "blob",
            &format!("{}:{}", head.0, task_path("a")),
        ],
    );
    assert_eq!(file + "\n", canonical(&task("a")));
    let snapshot = store(&repos.bob).read().unwrap();
    assert_eq!(snapshot.tasks, vec![task("a")]);
    assert_eq!(snapshot.quarantined, vec![]);
}

#[test]
fn a_hand_edited_task_file_is_set_aside_and_carried() {
    // The canonical file with its description edited by hand into what
    // quecto never writes: two blank lines.
    let edited = canonical(&task("a")).replace("Do it.", "Do it.\n\n\nNow.");
    let (repos, snapshot) = hand_edited(&[("a.md", edited.into_bytes())]);
    assert_eq!(ids(&snapshot), vec!["b"]);
    let (file, kept) = aside(&snapshot, &task_path("a"));
    assert_eq!(file.key, Some(ArtifactKey::Task(Slug::parse("a").unwrap())));
    assert!(kept, "{snapshot:?}");
    carried_and_blocking(&repos, &snapshot, &task_path("a"), "a");
}

#[test]
fn a_newer_schema_task_file_is_set_aside_with_upgrade_reason_and_carried() {
    let marker = format!("quecto-task schema={}", TaskCodec::SCHEMA);
    let newer = canonical(&task("a")).replace(
        &marker,
        &format!("quecto-task schema={}", TaskCodec::SCHEMA + 1),
    );
    let (repos, snapshot) = hand_edited(&[("a.md", newer.into_bytes())]);
    let (file, kept) = aside(&snapshot, &task_path("a"));
    assert!(file.reason.contains("upgrade quecto"), "{}", file.reason);
    assert_eq!(file.id().map(Slug::as_str), Some("a"));
    assert!(kept);
    carried_and_blocking(&repos, &snapshot, &task_path("a"), "a");
}

/// A board older quecto wrote holds each task as `<id>.json` and a
/// read-only `<id>.md` summary beside it: both are set aside under the id
/// and carried, the summary named as one, and clearing both frees the id.
#[test]
fn a_legacy_json_task_file_is_set_aside_kept_and_blocks_its_id() {
    let legacy = br#"{"schema": 3, "id": "old", "title": "Old"}"#.to_vec();
    let summary = b"# Old\n\n**old** \xc2\xb7 task \xc2\xb7 ready\n".to_vec();
    let (repos, snapshot) = hand_edited(&[("old.json", legacy), ("old.md", summary)]);
    assert_eq!(ids(&snapshot), vec!["b"]);
    let (json, kept) = aside(&snapshot, "tasks/old.json");
    assert_eq!(json.id().map(Slug::as_str), Some("old"));
    assert!(json.reason.contains("JSON"), "{}", json.reason);
    assert!(kept);
    let (md, kept) = aside(&snapshot, &task_path("old"));
    assert_eq!(md.id().map(Slug::as_str), Some("old"));
    assert!(
        md.reason
            .contains("a summary older quecto wrote beside its JSON record"),
        "{}",
        md.reason
    );
    assert!(kept);
    carried_and_blocking(&repos, &snapshot, "tasks/old.json", "old");
    // Clearing both, one by one, frees the id.
    let bob = store(&repos.bob);
    let mut head = bob.read().unwrap().head;
    for path in ["tasks/old.json", "tasks/old.md"] {
        let board = bob.read().unwrap();
        let (file, _) = aside(&board, path);
        head = written(bob.clear_set_aside(
            &head,
            &SetAsideClear {
                path: path.into(),
                object: file.object.clone(),
                repeated: Default::default(),
            },
            "board: clear old",
        ));
    }
    written(bob.write_tasks(&head, &[task("old")], "board: add old"));
}

#[test]
fn a_file_naming_another_task_is_set_aside() {
    let (repos, snapshot) = hand_edited(&[("x.md", canonical(&task("y")).into_bytes())]);
    assert_eq!(ids(&snapshot), vec!["b"]);
    let (file, kept) = aside(&snapshot, &task_path("x"));
    assert_eq!(file.id().map(Slug::as_str), Some("x"));
    assert!(file.reason.contains("\"y\""), "{}", file.reason);
    assert!(kept);
    carried_and_blocking(&repos, &snapshot, &task_path("x"), "x");
}
