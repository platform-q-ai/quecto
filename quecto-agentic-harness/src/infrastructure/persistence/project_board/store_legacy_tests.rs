//! Boards written before the task files became markdown (#2527 review): a
//! task file beside a file set aside under its id is set aside with it and
//! freed by clearing that file; a board read past what a write leaves is
//! cleared, never grown; a reason that quotes a file is capped.
use super::*;
use crate::application::project_board::ports::{KeepCopy, SetAsideClear};
use crate::infrastructure::persistence::project_board::board_tree::MAX_READ_ENTRIES;
use std::collections::BTreeSet;

fn clearing(path: &str, object: &str, repeated: &[(&str, KeepCopy)]) -> SetAsideClear {
    SetAsideClear {
        path: path.into(),
        object: object.into(),
        repeated: repeated
            .iter()
            .map(|(path, keep)| ((*path).to_string(), keep.clone()))
            .collect(),
    }
}

fn oid_of(repo: &Path, bytes: &[u8]) -> String {
    sh_with(repo, &["hash-object", "--stdin"], bytes)
}

fn task_md(id: &str) -> Object {
    Object::Blob(encode(&task(id)).unwrap())
}

/// The files set aside, by path, with the id each is under.
fn aside(snapshot: &BoardSnapshot) -> Vec<(&str, Option<&str>)> {
    snapshot
        .quarantined
        .iter()
        .map(|file| (file.path.as_str(), file.id().map(Slug::as_str)))
        .collect()
}

/// A task file in the JSON form older quecto wrote.
const LEGACY: &[u8] = br#"{"schema": 3, "id": "b", "title": "Old"}"#;

#[test]
fn a_task_file_beside_its_legacy_json_is_set_aside_until_that_is_cleared() {
    let repos = Repos::new();
    created(&store(&repos.alice));
    let head = raw_board(
        &repos.remote(),
        &[
            ("100644", "b.json", Object::Blob(LEGACY.to_vec())),
            ("100644", "b.md", task_md("b")),
        ],
        &[&repos.bob],
    );
    let bob = store(&repos.bob);
    let snapshot = bob.read().unwrap();
    assert_eq!(ids(&snapshot), Vec::<&str>::new());
    assert_eq!(
        aside(&snapshot),
        [("tasks/b.json", Some("b")), ("tasks/b.md", Some("b"))]
    );
    assert!(
        snapshot.quarantined.iter().all(|file| file.carried),
        "{:?}",
        snapshot.quarantined
    );
    let refused = bob.write_tasks(&head, &[task("b")], "board: write b");
    assert!(matches!(refused, Ok(BoardWrite::Refused(_))), "{refused:?}");
    let json = oid_of(&repos.bob, LEGACY);
    let cleared = written(bob.clear_set_aside(
        &head,
        &clearing("tasks/b.json", &json, &[]),
        "board: clear b.json",
    ));
    let snapshot = bob.read().unwrap();
    assert_eq!((&snapshot.head, ids(&snapshot)), (&cleared, vec!["b"]));
    assert_eq!(snapshot.quarantined, vec![]);
}

#[test]
fn a_task_file_beside_a_json_listed_twice_is_set_aside_until_that_is_cleared() {
    let repos = Repos::new();
    created(&store(&repos.alice));
    let head = raw_board(
        &repos.remote(),
        &[
            ("100644", "b.json", Object::Blob(b"one".to_vec())),
            ("100644", "b.json", Object::Blob(b"two".to_vec())),
            ("100644", "b.md", task_md("b")),
        ],
        &[&repos.bob],
    );
    let bob = store(&repos.bob);
    let snapshot = bob.read().unwrap();
    assert_eq!(ids(&snapshot), Vec::<&str>::new());
    assert!(aside(&snapshot).contains(&("tasks/b.md", Some("b"))));
    let refused = bob.write_tasks(&head, &[task("c")], "board: add c");
    assert!(matches!(refused, Ok(BoardWrite::Refused(_))), "{refused:?}");
    let [one, two] = [&b"one"[..], b"two"].map(|bytes| oid_of(&repos.bob, bytes));
    let seen = BTreeSet::from([one.clone(), two.clone()]);
    let cleared = written(bob.clear_set_aside(
        &head,
        &clearing(
            "tasks/b.json",
            &one,
            &[("tasks/b.json", KeepCopy::Drop(seen))],
        ),
        "board: clear b.json",
    ));
    let snapshot = bob.read().unwrap();
    assert_eq!((&snapshot.head, ids(&snapshot)), (&cleared, vec!["b"]));
}

/// A board written before the task files became markdown holds a JSON
/// file and a summary per task: one of twice the entries a write leaves is
/// still read and cleared file by file, but never grown.
#[test]
fn a_legacy_board_of_twice_the_write_cap_is_read_and_cleared_but_not_grown() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    let head = created(&alice);
    written(alice.write_tasks(&head, &[task("a")], "board: add a"));
    let blob = sh_with(&repos.remote(), &["hash-object", "-w", "--stdin"], b"x");
    let many: Vec<(String, String)> = (1..MAX_READ_ENTRIES)
        .map(|n| (format!("n{n}.md"), format!("100644 blob {blob}")))
        .collect();
    let many: Vec<(&str, String)> = many
        .iter()
        .map(|(name, meta)| (name.as_str(), meta.clone()))
        .collect();
    let full = hand_edit(&repos.remote(), &many);
    let bob = store(&repos.bob);
    let snapshot = bob.read().unwrap();
    assert_eq!(snapshot.quarantined.len(), MAX_READ_ENTRIES - 1);
    let grown = bob.write_tasks(&full, &[task("c")], "board: add c");
    assert!(
        matches!(&grown, Err(BoardStoreError::InvalidWrite(why)) if why.contains(&format!("at most {MAX_TASK_ENTRIES}"))),
        "{grown:?}"
    );
    written(bob.clear_set_aside(
        &full,
        &clearing("tasks/n1.md", &blob, &[]),
        "board: clear n1",
    ));
}

/// A reason may quote a file's own text (an id it refuses), cleaned and
/// capped as a quoted message is: past the cap it ends in " …".
#[test]
fn a_set_aside_reason_is_capped() {
    let repos = Repos::new();
    created(&store(&repos.alice));
    let text = String::from_utf8(encode(&task("long")).unwrap()).unwrap();
    let planted = text.replacen(
        "| Id | long |",
        &format!("| Id | {} |", "k".repeat(3 * 1024)),
        1,
    );
    assert_ne!(planted, text, "the id cell is planted");
    raw_board(
        &repos.remote(),
        &[("100644", "long.md", Object::Blob(planted.into_bytes()))],
        &[],
    );
    let reason = store(&repos.bob).read().unwrap().quarantined[0]
        .reason
        .clone();
    assert!(reason.contains("kkkk"), "it quotes the id: {reason}");
    assert!(
        reason.chars().count() <= MAX_QUOTED_CHARS + 2 && reason.ends_with(" …"),
        "{} characters",
        reason.chars().count()
    );
}
