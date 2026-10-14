//! Names listed more than once in a folder besides `tasks/` (#2481 S10b):
//! one clear resolves every such name, in `tasks/` and in every folder,
//! keeping the copy it names (or none), and pushes no tree a remote that
//! checks its objects refuses. A write that writes nothing in such a folder
//! carries it as its very tree; a write into it waits for the clear.
use super::*;
use crate::application::project_board::ports::KeepCopy;
use std::collections::BTreeSet;

fn seen(objects: &[&str]) -> BTreeSet<String> {
    objects.iter().map(|object| (*object).to_string()).collect()
}

/// The clear of `object` at `path`, resolving each of `repeated`.
fn resolving(path: &str, object: &str, repeated: &[(&str, KeepCopy)]) -> SetAsideClear {
    SetAsideClear {
        path: path.into(),
        object: object.into(),
        repeated: repeated
            .iter()
            .map(|(path, keep)| ((*path).to_string(), keep.clone()))
            .collect(),
    }
}

fn refused(outcome: Result<BoardWrite, BoardStoreError>) -> String {
    match outcome {
        Ok(BoardWrite::Refused(why)) => why,
        other => panic!("not refused: {other:?}"),
    }
}

/// `name` listed twice in a folder: its kind's file `good`, and a copy
/// that is no file of it.
fn twice(name: &str, good: Vec<u8>) -> Node {
    Node::Tree(vec![regular(name, good), regular(name, b"{".to_vec())])
}

/// A clear keeps the copy it names of a name listed twice in a folder and
/// drops the other; the folder is rebuilt with that copy alone, which a
/// remote that checks its objects takes, and the file is read again.
#[test]
fn a_clear_keeps_the_copy_it_names_of_a_name_listed_twice_in_a_folder() {
    let repos = fresh();
    let one = bytes::<RunCodec>(&run("a", 1));
    let head = board_of(&repos, |_| {
        vec![
            ("40000", "tasks", Node::Tree(vec![])),
            ("40000", "runs", twice("a.1.md", one.clone())),
        ]
    });
    let remote = repos.remote();
    sh(&remote, &["config", "receive.fsckObjects", "true"]);
    let bob = store(&repos.bob);
    let board = bob.read().unwrap();
    let key = ArtifactKey::Run(key_slug("a"), 1);
    assert_eq!(
        aside(&board),
        [
            ("runs/a.1.md", Some(&key), false),
            ("runs/a.1.md", Some(&key), false)
        ]
    );
    let (good, bad) = (at(&remote, &head, "runs/a.1.md"), blob_in(&repos.bob, b"{"));
    let copies = [good.as_str(), bad.as_str()];
    let keep = KeepCopy::Object {
        keep: good.clone(),
        seen: seen(&copies),
    };
    let clear = resolving("runs/a.1.md", &bad, &[("runs/a.1.md", keep)]);
    let cleared = written(bob.clear_set_aside(&head, &clear, "board: clear run a 1"));
    let rows = sh(&remote, &["ls-tree", &format!("{}:runs", cleared.0)]);
    assert_eq!(rows, format!("100644 blob {good}\ta.1.md"));
    let board = store(&repos.alice).read().unwrap();
    assert_eq!(board.runs, [run("a", 1)]);
    assert!(board.quarantined.is_empty(), "{:?}", board.quarantined);
}

/// While a name is listed twice in `plans/`, a write elsewhere lands and
/// carries `plans/` as its very tree (a remote that checks its objects
/// takes the push: it already holds that tree); a write into `plans/` is
/// refused, naming the clear that mends it.
#[test]
fn a_folder_with_a_name_listed_twice_is_carried_and_waits_for_its_clear() {
    let repos = fresh();
    let one = bytes::<PlanCodec>(&plan("a", 1));
    let head = board_of(&repos, |_| {
        vec![
            ("40000", "tasks", Node::Tree(vec![])),
            ("40000", "plans", twice("a.1.md", one.clone())),
        ]
    });
    let remote = repos.remote();
    sh(&remote, &["config", "receive.fsckObjects", "true"]);
    let bob = store(&repos.bob);
    let moved = written(bob.write_tasks(&head, &[task("b")], "board: add b"));
    assert_eq!(at(&remote, &moved, "plans"), at(&remote, &head, "plans"));
    let runs = [BoardArtifact::Run(run("a", 1))];
    let moved = written(bob.write(&moved, &runs, "board: run a"));
    let plans = [BoardArtifact::Plan(plan("a", 2))];
    let why = refused(bob.write(&moved, &plans, "board: plan a 2"));
    assert!(
        why.starts_with(&format!(
            "plans/a.1.md is named more than once in board commit {}",
            moved.0
        )),
        "{why}"
    );
    assert!(
        why.contains("no write in plans/ can carry it forward")
            && why.contains("clear it (clear set-aside)"),
        "{why}"
    );
}

/// One clear resolves every name listed twice, in `tasks/` and in each
/// folder: a clear that leaves any unresolved is refused, naming it; one
/// that resolves them all lands, keeping or dropping as it says, and a
/// folder left empty is no folder.
#[test]
fn one_clear_resolves_names_listed_twice_in_tasks_and_every_folder() {
    let repos = fresh();
    let head = board_of(&repos, |_| {
        vec![
            (
                "40000",
                "tasks",
                twice("dup.md", encode(&task("dup")).unwrap()),
            ),
            (
                "40000",
                "plans",
                twice("a.1.md", bytes::<PlanCodec>(&plan("a", 1))),
            ),
            (
                "40000",
                "runs",
                twice("a.1.md", bytes::<RunCodec>(&run("a", 1))),
            ),
        ]
    });
    let remote = repos.remote();
    sh(&remote, &["config", "receive.fsckObjects", "true"]);
    let bad = blob_in(&repos.bob, b"{");
    let both = |path: &str| [at(&remote, &head, path), bad.clone()];
    let (dup, plans, runs) = (
        both("tasks/dup.md"),
        both("plans/a.1.md"),
        both("runs/a.1.md"),
    );
    let all_of = |pair: &[String; 2]| pair.iter().cloned().collect::<BTreeSet<String>>();
    let keep = |pair: &[String; 2]| KeepCopy::Object {
        keep: pair[0].clone(),
        seen: all_of(pair),
    };
    let bob = store(&repos.bob);
    let tasks_only = resolving("tasks/dup.md", &bad, &[("tasks/dup.md", keep(&dup))]);
    let why = refused(bob.clear_set_aside(&head, &tasks_only, "board: clear"));
    assert!(
        why.contains("plans/a.1.md") && why.contains("runs/a.1.md"),
        "{why}"
    );
    let all = resolving(
        "tasks/dup.md",
        &bad,
        &[
            ("tasks/dup.md", keep(&dup)),
            ("plans/a.1.md", KeepCopy::Drop(all_of(&plans))),
            ("runs/a.1.md", keep(&runs)),
        ],
    );
    let cleared = written(bob.clear_set_aside(&head, &all, "board: clear every name"));
    let root = sh(&remote, &["ls-tree", "--name-only", &cleared.0]);
    assert!(!root.lines().any(|name| name == "plans"), "{root}");
    let board = store(&repos.alice).read().unwrap();
    assert_eq!(ids(&board), ["dup"]);
    assert_eq!(board.runs, [run("a", 1)]);
    assert!(
        board.plans.is_empty() && board.quarantined.is_empty(),
        "{board:?}"
    );
}

/// A clear that keeps a copy is a write a read must still take: keeping a
/// plan that takes `plans/` past its bytes is invalid, naming the drop
/// instead, and a clear dropping every copy lands.
#[test]
fn a_clear_keeping_a_copy_past_its_folders_bytes_is_invalid() {
    let repos = fresh();
    let cap = vec![b'x'; 1_536 * 1024];
    let head = board_of(&repos, |_| {
        let mut files: Vec<(&'static str, String, Vec<u8>)> = (1..=15)
            .map(|n| ("100644", format!("a.{n}.md"), cap.clone()))
            .collect();
        files.push(regular("b.1.md", b"x".to_vec()));
        files.push(regular("c.1.md", cap.clone()));
        files.push(regular("c.1.md", b"{".to_vec()));
        vec![
            ("40000", "tasks", Node::Tree(vec![])),
            ("40000", "plans", Node::Tree(files)),
            (
                "40000",
                "runs",
                twice("a.1.md", bytes::<RunCodec>(&run("a", 1))),
            ),
        ]
    });
    let remote = repos.remote();
    let (big, bad) = (
        at(&remote, &head, "plans/a.1.md"),
        blob_in(&repos.bob, b"{"),
    );
    let copies = seen(&[big.as_str(), bad.as_str()]);
    let one = at(&remote, &head, "runs/a.1.md");
    let runs = seen(&[one.as_str(), bad.as_str()]);
    let bob = store(&repos.bob);
    let keep = KeepCopy::Object {
        keep: big.clone(),
        seen: copies.clone(),
    };
    let keep_run = KeepCopy::Object {
        keep: one.clone(),
        seen: runs.clone(),
    };
    // Only the copy kept in the folder past its bytes is named.
    let clear = resolving(
        "plans/c.1.md",
        &bad,
        &[("plans/c.1.md", keep), ("runs/a.1.md", keep_run.clone())],
    );
    let past = bob.clear_set_aside(&head, &clear, "board: keep c 1");
    assert!(
        matches!(&past, Err(BoardStoreError::InvalidWrite(why)) if why.contains("bytes of plan files") && why.ends_with(", once this clear keeps its copy of plans/c.1.md: keep a smaller copy, or drop every copy instead")),
        "{past:?}"
    );
    let clear = resolving(
        "plans/c.1.md",
        &bad,
        &[
            ("plans/c.1.md", KeepCopy::Drop(copies)),
            ("runs/a.1.md", keep_run),
        ],
    );
    let cleared = written(bob.clear_set_aside(&head, &clear, "board: drop c 1"));
    assert_eq!(names(&remote, &cleared, "plans").len(), 16);
}

/// A refusal names at most eight kept copies, then how many more.
#[test]
fn a_refusal_names_eight_kept_copies_and_how_many_more() {
    let repos = fresh();
    let one = bytes::<PlanCodec>(&plan("a", 1));
    let head = board_of(&repos, |_| {
        let files = (1..=9)
            .flat_map(|n| {
                [
                    ("100644", format!("a.{n}.md"), one.clone()),
                    ("100644", format!("a.{n}.md"), b"{".to_vec()),
                ]
            })
            .collect();
        vec![
            ("40000", "tasks", Node::Tree(vec![])),
            ("40000", "plans", Node::Tree(files)),
        ]
    });
    let remote = repos.remote();
    let (good, bad) = (
        at(&remote, &head, "plans/a.1.md"),
        blob_in(&repos.bob, b"{"),
    );
    let keep = KeepCopy::Object {
        keep: good.clone(),
        seen: seen(&[good.as_str(), bad.as_str()]),
    };
    let paths: Vec<String> = (1..=9).map(|n| format!("plans/a.{n}.md")).collect();
    let all: Vec<(&str, KeepCopy)> = paths
        .iter()
        .map(|path| (path.as_str(), keep.clone()))
        .collect();
    let mut bob = store(&repos.bob);
    bob.limits.board_bytes = one.len();
    let past = bob.clear_set_aside(&head, &resolving("plans/a.1.md", &bad, &all), "board: keep");
    let shown = paths[..8].join(", ");
    assert!(
        matches!(&past, Err(BoardStoreError::InvalidWrite(why)) if why.ends_with(&format!(", once this clear keeps its copies of {shown} and 1 more: keep a smaller copy, or drop every copy instead"))),
        "{past:?}"
    );
}

/// A kept copy a read reads counts towards the board's bytes in all: one
/// that takes the board past them is invalid, naming it. A kept copy whose
/// name spells no key is never read, so it counts nothing.
#[test]
fn only_a_kept_copy_a_read_reads_counts_towards_the_boards_bytes() {
    let repos = fresh();
    let one = bytes::<PlanCodec>(&plan("a", 1));
    let notes = vec![b'n'; 4096];
    let head = board_of(&repos, |_| {
        let mut files = vec![regular("a.1.md", one.clone())];
        files.extend([
            regular("c.1.md", one.clone()),
            regular("c.1.md", b"{".to_vec()),
            regular("notes.md", notes.clone()),
            regular("notes.md", b"{".to_vec()),
        ]);
        vec![
            ("40000", "tasks", Node::Tree(vec![])),
            ("40000", "plans", Node::Tree(files)),
        ]
    });
    let remote = repos.remote();
    let bad = blob_in(&repos.bob, b"{");
    let (plan_copy, note_copy) = (
        at(&remote, &head, "plans/a.1.md"),
        blob_in(&repos.bob, &notes),
    );
    let mut bob = store(&repos.bob);
    bob.limits.board_bytes = one.len() + 10;
    let pair = |good: &str| seen(&[good, bad.as_str()]);
    let keep = |good: &str| KeepCopy::Object {
        keep: good.to_string(),
        seen: pair(good),
    };
    let keeping_the_plan = resolving(
        "plans/c.1.md",
        &bad,
        &[
            ("plans/c.1.md", keep(&plan_copy)),
            ("plans/notes.md", KeepCopy::Drop(pair(&note_copy))),
        ],
    );
    let past = bob.clear_set_aside(&head, &keeping_the_plan, "board: keep c 1");
    assert!(
        matches!(&past, Err(BoardStoreError::InvalidWrite(why)) if why.starts_with("the board would hold") && why.ends_with(", once this clear keeps its copy of plans/c.1.md: keep a smaller copy, or drop every copy instead")),
        "{past:?}"
    );
    let keeping_the_notes = resolving(
        "plans/c.1.md",
        &bad,
        &[
            ("plans/c.1.md", KeepCopy::Drop(pair(&plan_copy))),
            ("plans/notes.md", keep(&note_copy)),
        ],
    );
    let cleared = written(bob.clear_set_aside(&head, &keeping_the_notes, "board: keep notes"));
    assert_eq!(names(&remote, &cleared, "plans"), ["a.1.md", "notes.md"]);
}
