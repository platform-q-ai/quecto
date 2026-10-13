//! Writing the board's other folders (#2481 S10a): each artifact lands as
//! its kind's file under the name a read spells, every other file is
//! carried by its object, a folder a write leaves as it was is carried as
//! its very tree, and what a write cannot yet do safely waits for a clear
//! across folders (S10b).
use super::*;
use crate::application::project_board::ports::BoardArtifact;
use crate::infrastructure::persistence::project_board::artifacts::file_of;

/// The bytes of `path` at `commit` in `repo`.
fn blob_at(repo: &Path, commit: &BoardHead, path: &str) -> Vec<u8> {
    let oid = at(repo, commit, path);
    let output = Command::new("git")
        .current_dir(repo)
        .args(["cat-file", "blob", &oid])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .envs([
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{path}");
    output.stdout
}

/// The names of the folder `folder` at `commit` in `repo`.
fn names(repo: &Path, commit: &BoardHead, folder: &str) -> Vec<String> {
    let listing = sh(
        repo,
        &["ls-tree", "--name-only", &format!("{}:{folder}", commit.0)],
    );
    listing.lines().map(str::to_string).collect()
}

/// A write lands each artifact in its kind's folder as the very bytes its
/// codec makes, carries every other file of a folder it writes in by its
/// object, and carries each folder it leaves as it was (and the archive)
/// as its very tree.
#[test]
fn a_write_lands_each_file_in_its_folder_and_carries_the_rest_by_object() {
    let repos = fresh();
    let head = board_of(&repos, |repo| {
        let mut rows = every_kind();
        let old = vec![("100644", "old.txt".into(), blob_in(repo, b"anything"))];
        rows.push(("40000", "archive", Node::Made(old)));
        rows
    });
    let mut edited = run("a", 1).fields().clone();
    edited.plan_version = 2;
    let artifacts = [
        BoardArtifact::Plan(plan("a", 2)),
        BoardArtifact::Run(crate::domain::project_board::entities::run::Run::new(edited).unwrap()),
    ];
    let bob = store(&repos.bob);
    let moved = written(bob.write(&head, &artifacts, "board: plan a again, run a"));
    let remote = repos.remote();
    assert_eq!(
        blob_at(&remote, &moved, "plans/a.2.md"),
        file_of(&artifacts[0]).unwrap()
    );
    assert_eq!(
        blob_at(&remote, &moved, "runs/a.1.md"),
        file_of(&artifacts[1]).unwrap()
    );
    let carried = [
        "plans/a.1.md",
        "tasks",
        "changes",
        "reviews",
        "discussions",
        ARCHIVE,
    ];
    for path in carried {
        assert_eq!(
            at(&remote, &moved, path),
            at(&remote, &head, path),
            "{path}"
        );
    }
    let board = store(&repos.alice).read().unwrap();
    assert_eq!(board.plans.len(), 2);
    assert_eq!(board.runs, [run_of(&artifacts[1])]);
    assert!(board.quarantined.is_empty(), "{:?}", board.quarantined);
}

fn run_of(artifact: &BoardArtifact) -> crate::domain::project_board::entities::run::Run {
    match artifact {
        BoardArtifact::Run(run) => run.clone(),
        other => panic!("not a run: {other:?}"),
    }
}

/// A write naming a key whose file is set aside is refused; one beside it
/// lands, carrying the file set aside as it is and dropping strays, in a
/// folder it writes in and in one it leaves as it was alike, with no
/// folder left empty.
#[test]
fn a_write_beside_a_file_set_aside_carries_it_and_drops_strays() {
    let repos = fresh();
    let head = board_of(&repos, |_| {
        vec![
            ("40000", "tasks", Node::Tree(vec![])),
            (
                "40000",
                "plans",
                Node::Tree(vec![
                    regular("a.2.md", bytes::<PlanCodec>(&plan("a", 1))),
                    regular("notes.txt", b"notes".to_vec()),
                ]),
            ),
            (
                "40000",
                "runs",
                Node::Tree(vec![
                    regular("a.1.md", bytes::<RunCodec>(&run("a", 1))),
                    regular("notes.txt", b"notes".to_vec()),
                ]),
            ),
            (
                "40000",
                "changes",
                Node::Tree(vec![regular("notes.txt", b"notes".to_vec())]),
            ),
        ]
    });
    let bob = store(&repos.bob);
    let refused = bob.write(&head, &[BoardArtifact::Plan(plan("a", 2))], "board: plan a");
    assert!(
        matches!(&refused, Ok(BoardWrite::Refused(why)) if why.starts_with("plans/a.2.md is set aside: holds plan 1")),
        "{refused:?}"
    );
    let moved = written(bob.write(&head, &[BoardArtifact::Plan(plan("a", 3))], "board: plan a"));
    let remote = repos.remote();
    assert_eq!(names(&remote, &moved, "plans"), ["a.2.md", "a.3.md"]);
    assert_eq!(
        at(&remote, &moved, "plans/a.2.md"),
        at(&remote, &head, "plans/a.2.md")
    );
    assert_eq!(names(&remote, &moved, "runs"), ["a.1.md"]);
    let root = sh(&remote, &["ls-tree", "--name-only", &moved.0]);
    assert!(!root.lines().any(|name| name == "changes"), "{root}");
    let board = store(&repos.alice).read().unwrap();
    let a2 = ArtifactKey::Plan(key_slug("a"), 2);
    assert_eq!(aside(&board), [("plans/a.2.md", Some(&a2), true)]);
    assert!(reason(&board, "plans/a.2.md").ends_with("not plan 2 of task a"));
}

/// While a name outside `tasks/` is listed twice, nothing is written or
/// cleared; a clear of a file set aside outside `tasks/` waits too. A clear
/// in `tasks/` drops strays outside it, as a write does.
#[test]
fn what_a_write_cannot_yet_do_waits_for_a_clear_across_folders() {
    let repos = fresh();
    let one = bytes::<RunCodec>(&run("a", 1));
    let head = board_of(&repos, |_| {
        vec![
            (
                "40000",
                "tasks",
                Node::Tree(vec![regular("junk.md", b"junk".to_vec())]),
            ),
            (
                "40000",
                "runs",
                Node::Tree(vec![
                    regular("a.1.md", one.clone()),
                    regular("a.1.md", one.clone()),
                ]),
            ),
        ]
    });
    let bob = store(&repos.bob);
    let held = |result: Result<BoardWrite, BoardStoreError>| {
        assert!(
            matches!(&result, Err(BoardStoreError::FolderSetAside(why)) if why.starts_with("runs/a.1.md is named more than once")),
            "{result:?}"
        );
    };
    held(bob.write_tasks(&head, &[task("b")], "board: add b"));
    let junk = bob.read().unwrap().quarantined[2].clone();
    assert_eq!(junk.path, "tasks/junk.md");
    let clear = SetAsideClear {
        path: junk.path.clone(),
        object: junk.object.clone(),
        repeated: BTreeMap::new(),
    };
    held(bob.clear_set_aside(&head, &clear, "board: clear junk"));
    assert_eq!(sh(&repos.remote(), &["rev-parse", BOARD_REF]), head.0);
    let head = board_of(&repos, |_| {
        vec![
            (
                "40000",
                "tasks",
                Node::Tree(vec![regular("junk.md", b"junk".to_vec())]),
            ),
            (
                "40000",
                "runs",
                Node::Tree(vec![
                    regular("a.2.md", one.clone()),
                    regular("notes.txt", b"notes".to_vec()),
                ]),
            ),
        ]
    });
    let board = bob.read().unwrap();
    let file = board
        .quarantined
        .iter()
        .find(|file| file.path == "runs/a.2.md")
        .unwrap();
    let outside = SetAsideClear {
        path: file.path.clone(),
        object: file.object.clone(),
        repeated: BTreeMap::new(),
    };
    let waits = bob.clear_set_aside(&head, &outside, "board: clear run 2");
    assert!(
        matches!(&waits, Err(BoardStoreError::FolderSetAside(why)) if why.starts_with("runs/a.2.md is set aside, and this quecto clears only")),
        "{waits:?}"
    );
    let cleared = written(bob.clear_set_aside(&head, &clear, "board: clear junk"));
    assert_eq!(names(&repos.remote(), &cleared, "runs"), ["a.2.md"]);
    let board = store(&repos.alice).read().unwrap();
    let paths: Vec<&str> = board.quarantined.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["runs/a.2.md"]);
}

/// A write leaves each folder within the entries a read takes of it: a
/// folder full of files it carries takes no more.
#[test]
fn a_write_keeps_a_folder_within_its_entries() {
    let repos = fresh();
    let head = board_of(&repos, |repo| {
        let oid = blob_in(repo, b"x");
        let rows = (1..=4_096)
            .map(|n| ("100644", format!("x{n}.md"), oid.clone()))
            .collect();
        vec![
            ("40000", "tasks", Node::Tree(vec![])),
            ("40000", "plans", Node::Made(rows)),
        ]
    });
    let bob = store(&repos.bob);
    let full = bob.write(&head, &[BoardArtifact::Plan(plan("a", 1))], "board: plan a");
    assert!(
        matches!(&full, Err(BoardStoreError::InvalidWrite(why)) if why.contains("its plans/ holds at most 4096")),
        "{full:?}"
    );
    written(bob.write(&head, &[BoardArtifact::Run(run("a", 1))], "board: run a"));
}
