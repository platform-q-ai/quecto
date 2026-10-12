//! Boards written before the task files became markdown, and the format
//! marker that keeps older builds off a markdown board (#2527 review): a
//! task file beside a file set aside under its id is set aside with it and
//! freed by clearing that file; a board read past what a write leaves is
//! cleared, never grown; a root without the marker is read as it is and
//! gains it on the next write; a newer marker asks for an upgrade.
use super::*;
use crate::application::project_board::ports::{KeepCopy, SetAsideClear};
use crate::infrastructure::persistence::project_board::board_tree::{FORMAT, MAX_READ_ENTRIES};
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
    assert_eq!(
        snapshot.kept.iter().map(String::as_str).collect::<Vec<_>>(),
        ["tasks/b.json", "tasks/b.md"]
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

/// A board root of `format` (a marker of these bytes, or none) beside the
/// README and an empty `tasks`, pushed over the board.
fn marked(repos: &Repos, format: Option<&[u8]>) {
    let remote = repos.remote();
    let tip = sh(&remote, &["rev-parse", BOARD_REF]);
    let readme = sh(&remote, &["rev-parse", &format!("{tip}:README.md")]);
    let mut root = format!("100644 blob {readme}\tREADME.md\n");
    if let Some(bytes) = format {
        let marker = sh_with(&remote, &["hash-object", "-w", "--stdin"], bytes);
        let tree = sh_with(
            &remote,
            &["mktree"],
            format!("100644 blob {marker}\tboard\n").as_bytes(),
        );
        root += &format!("040000 tree {tree}\tformat\n");
    }
    let root = sh_with(&remote, &["mktree"], root.as_bytes());
    let commit = sh(
        &remote,
        &["commit-tree", &root, "-p", &tip, "-m", "hand edit"],
    );
    sh(&remote, &["update-ref", BOARD_REF, &commit]);
}

#[test]
fn a_board_without_a_format_marker_is_read_and_the_next_write_adds_it() {
    let repos = Repos::new();
    created(&store(&repos.alice));
    marked(&repos, None);
    let bob = store(&repos.bob);
    let snapshot = bob.read().unwrap();
    let moved = written(bob.write_tasks(&snapshot.head, &[task("a")], "board: add a"));
    assert_eq!(
        sh(
            &repos.remote(),
            &["cat-file", "blob", &format!("{}:format/board", moved.0)]
        ) + "\n",
        String::from_utf8_lossy(FORMAT)
    );
}

#[test]
fn a_newer_format_asks_for_an_upgrade_and_a_malformed_one_clashes() {
    let repos = Repos::new();
    created(&store(&repos.alice));
    marked(&repos, Some(b"quecto-board 3\n"));
    let read = store(&repos.bob).read();
    assert!(
        matches!(&read, Err(BoardStoreError::Unavailable(why)) if why.contains("upgrade quecto")),
        "{read:?}"
    );
    for odd in [
        &b"quecto-board 1\n"[..],
        b"quecto-board 02\n",
        b"board",
        b"",
    ] {
        marked(&repos, Some(odd));
        let read = store(&repos.bob).read();
        assert!(
            matches!(&read, Err(BoardStoreError::Clash(_))),
            "{odd:?}: {read:?}"
        );
    }
    marked(&repos, Some(FORMAT));
    assert!(store(&repos.bob).read().is_ok());
}

/// The board shape the check as merged before the marker took (master
/// `0723d9ee6`, `board_tree.rs` `board_shape`): a root entry is a file, or
/// the `tasks` tree; the root holds a README or `tasks`. Kept here so the
/// marker's promise stays checked: that quecto takes a marked root for a
/// clash, so it never reads, and never writes over, a board whose tasks it
/// would not see.
fn merged_board_shape(root: &[(&str, &str)]) -> Result<(), String> {
    let fits = |(name, kind): &(&str, &str)| match *name {
        "tasks" => *kind == "tree",
        _ => *kind == "blob",
    };
    let ours = |(name, _): &(&str, &str)| matches!(*name, "README.md" | "tasks");
    match (
        root.iter().any(ours),
        root.iter().find(|entry| !fits(entry)),
    ) {
        (true, None) => Ok(()),
        (false, None) => Err("neither a README.md nor a tasks tree".into()),
        (_, Some((name, kind))) => Err(format!("`{name}` is a {kind}: a clash")),
    }
}

#[test]
fn the_merged_board_shape_takes_a_marked_root_for_a_clash() {
    let legacy = [("README.md", "blob"), ("tasks", "tree")];
    assert_eq!(merged_board_shape(&legacy), Ok(()));
    let marked = [("README.md", "blob"), ("format", "tree"), ("tasks", "tree")];
    assert!(merged_board_shape(&marked).is_err());
    // And a write of this quecto leaves such a root.
    let repos = Repos::new();
    let alice = store(&repos.alice);
    let head = created(&alice);
    let moved = written(alice.write_tasks(&head, &[task("a")], "board: add a"));
    let root = sh(&repos.remote(), &["ls-tree", &moved.0]);
    let root: Vec<(&str, &str)> = root
        .lines()
        .map(|row| {
            let (meta, name) = row.split_once('\t').unwrap();
            (name, meta.split(' ').nth(1).unwrap())
        })
        .collect();
    assert_eq!(root, marked);
}
