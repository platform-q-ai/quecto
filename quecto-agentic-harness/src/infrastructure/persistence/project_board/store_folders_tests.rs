//! The board's other folders (#2481 S9): `plans`, `runs`, `changes`,
//! `reviews` and `discussions` are read as their kinds, each file only as
//! what its name spells, and an `archive` tree is room left for later,
//! type-checked and never read. A write or a clear carries every folder
//! and the archive as they are, and none is made while a file outside
//! `tasks/` is set aside: only a quecto that writes those folders can
//! carry or drop it.
use super::format::{blob_in, raw_rows};
use super::*;
use crate::application::project_board::ports::{ArtifactKey, SetAsideClear};
use crate::domain::project_board::value_objects::slug::MAX_SLUG_BYTES;
use crate::infrastructure::persistence::project_board::artifacts::change::ChangeCodec;
use crate::infrastructure::persistence::project_board::artifacts::discussion::DiscussionCodec;
use crate::infrastructure::persistence::project_board::artifacts::plan::PlanCodec;
use crate::infrastructure::persistence::project_board::artifacts::review::ReviewCodec;
use crate::infrastructure::persistence::project_board::artifacts::run::RunCodec;
use crate::infrastructure::persistence::project_board::artifacts::{ArtifactCodec, encode as file};
use crate::infrastructure::persistence::project_board::board_tree::{ARCHIVE, FORMAT};
use crate::infrastructure::test_support::board_artifacts::{change, discussion, plan, review, run};

/// A row of a tree: its mode, its name and the object's id.
type Row = (&'static str, String, String);

/// A row of the root: its mode, its name and what it holds.
type RootRow = (&'static str, &'static str, Node);

/// A row of the root made fresh for each repository.
type MadeRow = (&'static str, &'static str, fn() -> Node);

/// The file `value` is, as bytes.
fn bytes<C: ArtifactCodec>(value: &C::Value) -> Vec<u8> {
    file::<C>(value).unwrap().into_bytes()
}

fn key_slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

/// What a row of the root holds, made in one repository.
enum Node {
    /// A tree of these rows (mode, name, bytes), each blob made here.
    Tree(Vec<(&'static str, String, Vec<u8>)>),
    /// A tree of rows already made (mode, name, object id).
    Made(Vec<Row>),
    Blob(Vec<u8>),
}

fn regular(name: &str, bytes: Vec<u8>) -> (&'static str, String, Vec<u8>) {
    ("100644", name.to_string(), bytes)
}

/// The board's head moved to a root of the README, a `format` tree whose
/// `board` reads `marker`, and `rows` (mode, name, node): the same commit
/// in the remote and in bob's clone (fetched there, as it were), since
/// git's transport refuses to send some such trees.
fn board_marked(repos: &Repos, marker: &[u8], rows: impl Fn(&Path) -> Vec<RootRow>) -> BoardHead {
    let mut head: Option<BoardHead> = None;
    for repo in [repos.remote(), repos.bob.clone()] {
        let format = raw_rows(&repo, &[("100644", "board", blob_in(&repo, marker))]);
        let mut root: Vec<Row> = vec![
            ("100644", "README.md".into(), blob_in(&repo, README)),
            ("40000", "format".into(), format),
        ];
        for (mode, name, node) in rows(&repo) {
            let oid = match node {
                Node::Tree(entries) => {
                    let made: Vec<Row> = entries
                        .into_iter()
                        .map(|(mode, name, bytes)| (mode, name, blob_in(&repo, &bytes)))
                        .collect();
                    tree_of(&repo, &made)
                }
                Node::Made(made) => tree_of(&repo, &made),
                Node::Blob(bytes) => blob_in(&repo, &bytes),
            };
            root.push((mode, name.to_string(), oid));
        }
        // git's order: a tree sorts as its name and a slash.
        root.sort_by_key(|(mode, name, _)| match *mode {
            "40000" => format!("{name}/"),
            _ => name.clone(),
        });
        let commit = raw_commit(&repo, &tree_of(&repo, &root));
        if let Some(head) = &head {
            assert_eq!(&commit, head, "the same commit in each repository");
        }
        head = Some(commit);
    }
    let head = head.unwrap();
    sh(&repos.remote(), &["update-ref", BOARD_REF, &head.0]);
    sh(
        &repos.bob,
        &["update-ref", "refs/remotes/origin/quecto/board", &head.0],
    );
    head
}

fn board_of(repos: &Repos, rows: impl Fn(&Path) -> Vec<RootRow>) -> BoardHead {
    board_marked(repos, FORMAT, rows)
}

fn tree_of(repo: &Path, rows: &[Row]) -> String {
    let rows: Vec<(&str, &str, String)> = rows
        .iter()
        .map(|(mode, name, oid)| (*mode, name.as_str(), oid.clone()))
        .collect();
    raw_rows(repo, &rows)
}

/// Task `a`, and one file of each other kind, each named as its folder
/// names it.
fn every_kind() -> Vec<RootRow> {
    let folder = |name: &'static str, file: &str, bytes: Vec<u8>| {
        ("40000", name, Node::Tree(vec![regular(file, bytes)]))
    };
    vec![
        folder("tasks", "a.md", encode(&task("a")).unwrap()),
        folder("plans", "a.1.md", bytes::<PlanCodec>(&plan("a", 1))),
        folder("runs", "a.1.md", bytes::<RunCodec>(&run("a", 1))),
        folder("changes", "a.1.md", bytes::<ChangeCodec>(&change("a", 1))),
        folder("reviews", "a.1.md", bytes::<ReviewCodec>(&review("a", 1))),
        folder(
            "discussions",
            "d.md",
            bytes::<DiscussionCodec>(&discussion("d")),
        ),
    ]
}

fn fresh() -> Repos {
    let repos = Repos::new();
    created(&store(&repos.alice));
    repos
}

/// Every set-aside file: path, key, carried.
fn aside(snapshot: &BoardSnapshot) -> Vec<(&str, Option<&ArtifactKey>, bool)> {
    snapshot
        .quarantined
        .iter()
        .map(|file| (file.path.as_str(), file.key.as_ref(), file.carried))
        .collect()
}

fn reason<'a>(snapshot: &'a BoardSnapshot, path: &str) -> &'a str {
    let file = snapshot.quarantined.iter().find(|file| file.path == path);
    &file.unwrap_or_else(|| panic!("{path} is set aside")).reason
}

#[test]
fn every_folder_is_read_as_its_kind() {
    let repos = fresh();
    board_of(&repos, |_| every_kind());
    let board = store(&repos.bob).read().unwrap();
    assert_eq!(ids(&board), ["a"]);
    assert_eq!(board.plans, [plan("a", 1)]);
    assert_eq!(board.runs, [run("a", 1)]);
    assert_eq!(board.changes, [change("a", 1)]);
    assert_eq!(board.reviews, [review("a", 1)]);
    assert_eq!(board.discussions, [discussion("d")]);
    assert!(board.quarantined.is_empty(), "{:?}", board.quarantined);
}

/// A file is read only as what its name spells: one holding another key,
/// another kind, a newer schema or no file at all is set aside under the
/// key its name spells and carried forward as it is.
#[test]
fn a_file_is_read_only_as_what_its_name_spells() {
    let repos = fresh();
    let newer = String::from_utf8(bytes::<ChangeCodec>(&change("a", 1)))
        .unwrap()
        .replace("quecto-change schema=1", "quecto-change schema=2");
    board_of(&repos, |_| {
        let folder = |name: &'static str, file: &str, bytes: Vec<u8>| {
            ("40000", name, Node::Tree(vec![regular(file, bytes)]))
        };
        vec![
            folder("plans", "a.2.md", bytes::<PlanCodec>(&plan("a", 1))),
            folder("runs", "a.1.md", bytes::<PlanCodec>(&plan("a", 1))),
            folder("changes", "a.1.md", newer.clone().into_bytes()),
            folder("reviews", "a.1.md", b"hello\n".to_vec()),
            folder(
                "discussions",
                "d.md",
                bytes::<DiscussionCodec>(&discussion("e")),
            ),
            ("40000", "tasks", Node::Tree(vec![])),
        ]
    });
    let board = store(&repos.bob).read().unwrap();
    let a = || key_slug("a");
    assert_eq!(
        aside(&board),
        [
            ("changes/a.1.md", Some(&ArtifactKey::Change(a(), 1)), true),
            (
                "discussions/d.md",
                Some(&ArtifactKey::Discussion(key_slug("d"))),
                true
            ),
            ("plans/a.2.md", Some(&ArtifactKey::Plan(a(), 2)), true),
            ("reviews/a.1.md", Some(&ArtifactKey::Review(a(), 1)), true),
            ("runs/a.1.md", Some(&ArtifactKey::Run(a(), 1)), true),
        ]
    );
    assert!(
        reason(&board, "plans/a.2.md").contains("holds plan 1 of task a, not plan 2 of task a")
    );
    assert!(reason(&board, "discussions/d.md").contains("holds discussion e, not discussion d"));
    assert!(reason(&board, "runs/a.1.md").starts_with("run file: "));
    assert!(reason(&board, "reviews/a.1.md").starts_with("review file: "));
    assert!(reason(&board, "changes/a.1.md").contains("upgrade quecto"));
    let read = [
        board.plans.len(),
        board.runs.len(),
        board.changes.len(),
        board.reviews.len(),
        board.discussions.len(),
    ];
    assert_eq!(read, [0; 5], "nothing set aside is read");
}

/// Names in a folder are judged as in `tasks/`: a name that is no file of
/// the folder's kind is a stray; an `.md` whose name spells no key is
/// carried with none; a file of a key's name that is not a regular file is
/// set aside under it; a gitlink is dropped. The longest name spells the
/// longest id and number.
#[test]
fn names_in_a_folder_are_judged_as_in_tasks() {
    let repos = fresh();
    let longest = format!("{}.999.md", "x".repeat(MAX_SLUG_BYTES));
    let too_long = format!("{}.999.md", "x".repeat(MAX_SLUG_BYTES + 1));
    let plan = bytes::<PlanCodec>(&plan("a", 3));
    board_of(&repos, |_| {
        let entries = vec![
            regular("notes.txt", b"notes".to_vec()),
            regular("a..b.txt", b"notes".to_vec()),
            regular("a.md", plan.clone()),
            regular("a.01.md", plan.clone()),
            regular("a.0.md", plan.clone()),
            regular("a.1000.md", plan.clone()),
            ("100755", "a.3.md".into(), plan.clone()),
            ("160000", "a.4.md".into(), b"a module".to_vec()),
            regular(&longest, plan.clone()),
            regular(&too_long, plan.clone()),
        ];
        vec![
            ("40000", "tasks", Node::Tree(vec![])),
            ("40000", "plans", Node::Tree(entries)),
            (
                "40000",
                "discussions",
                Node::Tree(vec![regular("d.1.md", b"x".to_vec())]),
            ),
        ]
    });
    let board = store(&repos.bob).read().unwrap();
    let plan_key = |slug: &str, n| ArtifactKey::Plan(key_slug(slug), n);
    let (three, most) = (plan_key("a", 3), plan_key(&"x".repeat(MAX_SLUG_BYTES), 999));
    let mut want = vec![
        ("discussions/d.1.md", None, true),
        ("plans/a..b.txt", None, false),
        ("plans/a.0.md", None, true),
        ("plans/a.01.md", None, true),
        ("plans/a.1000.md", None, true),
        ("plans/a.3.md", Some(&three), true),
        ("plans/a.4.md", None, false),
        ("plans/a.md", None, true),
        ("plans/notes.txt", None, false),
    ];
    want.push((format!("plans/{longest}").leak(), Some(&most), true));
    want.push((format!("plans/{too_long}").leak(), None, false));
    want.sort();
    let mut got = aside(&board);
    got.sort();
    assert_eq!(got, want);
    assert!(reason(&board, "plans/notes.txt").contains("only <task>.<n>.md belongs in plans/"));
    assert!(reason(&board, "plans/a.md").contains("not a plan name"));
    assert!(reason(&board, "plans/a.3.md").contains("not a regular file"));
    assert!(reason(&board, &format!("plans/{longest}")).contains("holds plan 3 of task a"));
    assert!(reason(&board, &format!("plans/{too_long}")).contains("not a name quecto keeps"));
    // This quecto writes no folder but `tasks/`: a reason says what is
    // wrong, never what a write would do.
    for file in &board.quarantined {
        assert!(
            !file.reason.contains("write") && !file.reason.contains("carried"),
            "{}: {}",
            file.path,
            file.reason
        );
    }
}

/// A name listed twice in a folder sets every copy aside under its key,
/// neither carried nor read.
#[test]
fn a_name_listed_twice_in_a_folder_sets_every_copy_aside() {
    let repos = fresh();
    board_of(&repos, |_| {
        let one = bytes::<RunCodec>(&run("a", 1));
        let twice = vec![regular("a.1.md", one.clone()), regular("a.1.md", one)];
        vec![
            ("40000", "tasks", Node::Tree(vec![])),
            ("40000", "runs", Node::Tree(twice)),
        ]
    });
    let board = store(&repos.bob).read().unwrap();
    let key = ArtifactKey::Run(key_slug("a"), 1);
    assert_eq!(
        aside(&board),
        [
            ("runs/a.1.md", Some(&key), false),
            ("runs/a.1.md", Some(&key), false)
        ]
    );
    assert!(board.runs.is_empty());
}

/// The object id of `path` at `commit` in `repo`.
fn at(repo: &Path, commit: &BoardHead, path: &str) -> String {
    sh(repo, &["rev-parse", &format!("{}:{path}", commit.0)])
}

const CARRIED: [&str; 6] = [
    "plans",
    "runs",
    "changes",
    "reviews",
    "discussions",
    "archive",
];

/// A write and a clear carry every folder and the archive as they are
/// (whatever the archive holds), and leave this quecto's marker.
#[test]
fn a_write_and_a_clear_carry_every_folder_and_the_archive_as_they_are() {
    let repos = fresh();
    let head = board_of(&repos, |repo| {
        let mut rows = every_kind();
        rows[0] = (
            "40000",
            "tasks",
            Node::Tree(vec![
                regular("a.md", encode(&task("a")).unwrap()),
                regular("junk.md", b"junk".to_vec()),
            ]),
        );
        let empty = raw_rows(repo, &[]);
        let archive = vec![
            ("100644", "old.txt".into(), blob_in(repo, b"anything")),
            ("40000", "plans".into(), empty),
        ];
        rows.push(("40000", "archive", Node::Made(archive)));
        rows
    });
    let remote = repos.remote();
    let bob = store(&repos.bob);
    let moved = written(bob.write_tasks(&head, &[task("b")], "board: add b"));
    for path in CARRIED {
        assert_eq!(
            at(&remote, &moved, path),
            at(&remote, &head, path),
            "{path}"
        );
    }
    assert_eq!(
        sh(
            &remote,
            &["cat-file", "blob", &format!("{}:format/board", moved.0)]
        ) + "\n",
        String::from_utf8_lossy(FORMAT)
    );
    let board = store(&repos.alice).read().unwrap();
    assert_eq!(ids(&board), ["a", "b"]);
    assert_eq!(board.plans, [plan("a", 1)]);
    assert_eq!(board.discussions, [discussion("d")]);
    let junk = board.quarantined[0].clone();
    assert_eq!(junk.path, "tasks/junk.md");
    let clear = SetAsideClear {
        path: junk.path,
        object: junk.object,
        repeated: BTreeMap::new(),
    };
    let cleared = written(bob.clear_set_aside(&moved, &clear, "board: clear junk"));
    for path in CARRIED {
        assert_eq!(
            at(&remote, &cleared, path),
            at(&remote, &head, path),
            "{path}"
        );
    }
    assert!(store(&repos.alice).read().unwrap().quarantined.is_empty());
}

/// While a file outside `tasks/` is set aside, nothing is written or
/// cleared: this quecto neither carries nor drops one.
#[test]
fn nothing_is_written_or_cleared_while_a_file_outside_tasks_is_set_aside() {
    let repos = fresh();
    let head = board_of(&repos, |_| {
        vec![
            (
                "40000",
                "tasks",
                Node::Tree(vec![regular("junk.md", b"junk".to_vec())]),
            ),
            (
                "40000",
                "plans",
                Node::Tree(vec![regular("a.2.md", bytes::<PlanCodec>(&plan("a", 1)))]),
            ),
        ]
    });
    let bob = store(&repos.bob);
    let board = bob.read().unwrap();
    let write = bob.write_tasks(&head, &[task("b")], "board: add b");
    assert!(
        matches!(&write, Err(BoardStoreError::FolderSetAside(why)) if why.contains("plans/a.2.md")),
        "{write:?}"
    );
    for file in &board.quarantined {
        let clear = SetAsideClear {
            path: file.path.clone(),
            object: file.object.clone(),
            repeated: BTreeMap::new(),
        };
        let cleared = bob.clear_set_aside(&head, &clear, "board: clear");
        assert!(
            matches!(&cleared, Err(BoardStoreError::FolderSetAside(why)) if why.contains("plans/a.2.md")),
            "{}: {cleared:?}",
            file.path
        );
    }
    assert_eq!(sh(&repos.remote(), &["rev-parse", BOARD_REF]), head.0);
}

fn tree() -> Node {
    Node::Tree(vec![])
}

fn stray() -> Node {
    Node::Blob(b"x".to_vec())
}

/// The board at a root of `rows` (mode, name, node) and an empty `tasks`.
fn read_root(rows: &[MadeRow]) -> Result<(), BoardStoreError> {
    let repos = fresh();
    let head = board_of(&repos, |_| {
        let mut all: Vec<RootRow> = rows
            .iter()
            .map(|(mode, name, node)| (*mode, *name, node()))
            .collect();
        all.push(("40000", "tasks", tree()));
        all
    });
    store(&repos.bob).board_at(&head.0).map(|_| ())
}

/// The root holds the six folders and an archive, each a tree once; any
/// other tree, or one of theirs that is not a tree, is no board.
#[test]
fn the_root_holds_the_folders_and_an_archive_each_a_tree_once() {
    let mut every: Vec<MadeRow> = CARRIED
        .iter()
        .map(|name| ("40000", *name, tree as fn() -> Node))
        .collect();
    every.push(("100644", "notes.txt", stray));
    assert_eq!(
        read_root(&every),
        Ok(()),
        "every folder, the archive and a stray file"
    );
    for name in ["src", "archives"] {
        let clash = read_root(&[("40000", name, tree)]);
        assert!(
            matches!(&clash, Err(BoardStoreError::Clash(_))),
            "{name}: {clash:?}"
        );
    }
    for name in CARRIED {
        let clash = read_root(&[("100644", name, stray)]);
        assert!(
            matches!(&clash, Err(BoardStoreError::Clash(_))),
            "{name}: {clash:?}"
        );
        // Named twice is damage, whatever the copies are: a tree and a
        // file are not taken for a clash.
        for copies in [
            [("40000", name, tree as fn() -> Node), ("40000", name, tree)],
            [("40000", name, tree), ("100644", name, stray)],
        ] {
            let twice = read_root(&copies);
            assert!(
                matches!(&twice, Err(BoardStoreError::Unavailable(why)) if why.contains(&format!("names {name} more than once"))),
                "{name}: {twice:?}"
            );
        }
    }
    // `tasks` too: `read_root` adds one tree of it.
    let twice = read_root(&[("100644", "tasks", stray)]);
    assert!(
        matches!(&twice, Err(BoardStoreError::Unavailable(why)) if why.contains("names tasks more than once")),
        "{twice:?}"
    );
}

/// A name that shows as nothing, `.`, `..`, or holds a `/` (a hand-made
/// tree's) is listed as `…` in its folder: one name a listing, an answer
/// and the index take back, so every listing still answers, and the index
/// holds without a rebuild.
#[test]
fn a_name_that_shows_as_no_name_is_listed_as_one() {
    use crate::application::project_board::ports::BoardListings;
    use crate::infrastructure::persistence::project_board::index::{BoardIndex, Refreshed};
    use crate::infrastructure::persistence::project_board::indexed_listings::IndexedBoardListings;
    let repos = fresh();
    let odd = || {
        ["\u{1}", "\u{1}.\u{1}", "\u{1}..\u{1}", ".", "x/y"]
            .map(|name| regular(name, b"odd".to_vec()))
            .to_vec()
    };
    board_of(&repos, |_| {
        let mut tasks = odd();
        tasks.push(regular("a.md", encode(&task("a")).unwrap()));
        vec![
            ("40000", "tasks", Node::Tree(tasks)),
            ("40000", "plans", Node::Tree(odd())),
        ]
    });
    let bob = Arc::new(store(&repos.bob));
    let snapshot = bob.read().unwrap();
    let paths: Vec<&str> = snapshot
        .quarantined
        .iter()
        .map(|f| f.path.as_str())
        .collect();
    assert_eq!(paths, [["plans/…"; 5], ["tasks/…"; 5]].concat());
    let index = BoardIndex::of(&bob).unwrap();
    let listings = IndexedBoardListings::new(index, bob.clone(), bob.clone());
    for _ in 0..2 {
        assert_eq!(listings.ready().unwrap().quarantined.len(), 10);
    }
    let index = BoardIndex::of(&bob).unwrap();
    assert_eq!(
        index.refresh(&*bob, &*bob),
        Ok(Refreshed::Unchanged {
            head: snapshot.head.0.clone()
        }),
        "the index holds: no rebuild"
    );
}

/// A partial clone that fetched the board without its trees and blobs
/// (`--filter=tree:0`) fetches what the folders and the archive miss, in
/// one call per tree level, then reads every kind and carries the archive.
#[test]
fn a_partial_clone_reads_every_folder_and_carries_the_archive() {
    let repos = fresh();
    let remote = repos.remote();
    for (key, value) in [
        ("uploadpack.allowFilter", "true"),
        ("uploadpack.allowAnySHA1InWant", "true"),
    ] {
        sh(&remote, &["config", key, value]);
    }
    let head = board_of(&repos, |repo| {
        let mut rows = every_kind();
        let old = vec![("100644", "old.txt".into(), blob_in(repo, b"anything"))];
        rows.push(("40000", "archive", Node::Made(old)));
        rows
    });
    let url = format!("file://{}", remote.display());
    sh(
        repos.dir.path(),
        &[
            "clone",
            "-q",
            "--filter=tree:0",
            "--no-single-branch",
            &url,
            "carol",
        ],
    );
    let carol = repos.dir.path().join("carol");
    let tree = format!("{}^{{tree}}", head.0);
    let held = Command::new("git")
        .current_dir(&carol)
        .args(["cat-file", "-e", &tree])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .envs([
            ("HOME", carol.as_os_str()),
            ("GIT_CONFIG_GLOBAL", "/dev/null".as_ref()),
            ("GIT_CONFIG_NOSYSTEM", "1".as_ref()),
            ("GIT_NO_LAZY_FETCH", "1".as_ref()),
        ])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(!held.success(), "the clone holds no tree of the board yet");
    let partial = store(&carol);
    let board = partial.read().unwrap();
    assert_eq!(board.reviews, [review("a", 1)]);
    assert_eq!(board.discussions, [discussion("d")]);
    let moved = written(partial.write_tasks(&head, &[task("b")], "board: add b"));
    assert_eq!(at(&remote, &moved, ARCHIVE), at(&remote, &head, ARCHIVE));
}

#[path = "store_folders_bounds_tests.rs"]
mod bounds;
