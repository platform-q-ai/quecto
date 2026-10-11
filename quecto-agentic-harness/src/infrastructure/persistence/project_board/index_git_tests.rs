//! The index against a real board branch: refreshes, a board that is gone,
//! and snapshots that land out of order.
use super::*;

fn sh(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .envs([
            ("HOME", "/nonexistent"),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ])
        .envs([("GIT_AUTHOR_NAME", "T"), ("GIT_AUTHOR_EMAIL", "t@x.io")])
        .envs([
            ("GIT_COMMITTER_NAME", "T"),
            ("GIT_COMMITTER_EMAIL", "t@x.io"),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string()
}

fn store(repo: &Path) -> GitBoardStore {
    let vars: Vec<(std::ffi::OsString, std::ffi::OsString)> = vec![
        ("PATH".into(), std::env::var_os("PATH").unwrap()),
        ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
    ];
    let identity = Identity {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    };
    let git = Git::new(repo, GitEnvironment::allowlisted(vars), identity).unwrap();
    GitBoardStore::new(git, "origin").unwrap()
}

struct Repo {
    dir: tempfile::TempDir,
    alice: GitBoardStore,
}

impl Repo {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sh(root, &["init", "-q", "--bare", "-b", "main", "remote.git"]);
        sh(root, &["clone", "-q", "remote.git", "alice"]);
        sh(
            &root.join("alice"),
            &["commit", "-q", "--allow-empty", "-m", "init"],
        );
        let alice = store(&root.join("alice"));
        Self { dir, alice }
    }

    fn remote(&self) -> std::path::PathBuf {
        self.dir.path().join("remote.git")
    }

    fn bootstrapped(&self) -> String {
        let Ok(BoardBootstrap::Created(BoardHead(head))) = self.alice.bootstrap() else {
            panic!("created")
        };
        head
    }

    fn add(&self, id: &str) -> BoardSnapshot {
        let head = self.alice.read().unwrap().head;
        let written = self.alice.write(
            &head,
            &[task(id, TaskStatus::Ready, None, &[])],
            &format!("board: add {id}"),
        );
        assert!(matches!(written, Ok(BoardWrite::Written(_))), "{written:?}");
        self.alice.read().unwrap()
    }
}

#[test]
fn refresh_rebuilds_from_the_board_branch_only_when_it_moves() {
    let repo = Repo::new();
    let root = repo.dir.path();
    sh(&root.join("alice"), &["worktree", "add", "-q", "../linked"]);
    let (alice, linked) = (&repo.alice, store(&root.join("linked")));
    let index = BoardIndex::of(alice).unwrap();
    let at = index.path().display().to_string();
    assert!(
        index
            .path()
            .ends_with("alice/.git/quecto/project-board.sqlite"),
        "{at}"
    );
    assert_eq!(
        BoardIndex::of(&linked).unwrap(),
        index,
        "one index per repository, shared by its worktrees"
    );
    assert_eq!(
        index.refresh(alice, alice),
        Err(IndexError::Store(BoardStoreError::NoBoard))
    );
    let head = repo.bootstrapped();
    assert_eq!(
        index.refresh(alice, alice),
        Ok(Refreshed::Rebuilt { head: head.clone() })
    );
    assert_eq!(
        index.refresh(&linked, &linked),
        Ok(Refreshed::Unchanged { head })
    );
    let moved = repo.add("a");
    assert_eq!(
        index.refresh(alice, alice),
        Ok(Refreshed::Rebuilt {
            head: moved.head.0.clone()
        })
    );
    assert_eq!(
        index.ready(),
        Ok(BoardListing {
            head: moved.head,
            tasks: moved.tasks,
            quarantined: vec![]
        })
    );
}

#[test]
fn a_board_that_is_gone_takes_the_index_with_it() {
    let repo = Repo::new();
    let index = BoardIndex::of(&repo.alice).unwrap();
    repo.bootstrapped();
    repo.add("a");
    index.refresh(&repo.alice, &repo.alice).unwrap();
    sh(
        &repo.remote(),
        &["update-ref", "-d", "refs/heads/quecto/board"],
    );
    assert_eq!(
        index.refresh(&repo.alice, &repo.alice),
        Err(IndexError::Store(BoardStoreError::NoBoard))
    );
    assert_eq!(index.ready(), Err(IndexError::NeedsRebuild));
    assert!(!index.path().exists());
}

#[test]
fn a_snapshot_read_before_the_board_moved_on_never_replaces_the_newer_index() {
    let repo = Repo::new();
    let index = BoardIndex::of(&repo.alice).unwrap();
    repo.bootstrapped();
    let (h2, h3) = (repo.add("a"), repo.add("b"));
    let rebuilt = |snapshot: &BoardSnapshot| {
        Ok(Refreshed::Rebuilt {
            head: snapshot.head.0.clone(),
        })
    };
    // B read H3 and lands first; A read H2 earlier and lands after.
    assert_eq!(index.rebuild(&h3, &repo.alice), rebuilt(&h3));
    assert_eq!(
        index.rebuild(&h2, &repo.alice),
        Ok(Refreshed::Newer {
            head: h3.head.0.clone()
        })
    );
    assert_eq!(index.head(), Ok(Some(h3.head.0.clone())));
    assert_eq!(ids(index.by_status(TaskStatus::Ready)), ["a", "b"]);
    // The board is reset to H2 (a force-push): H2 is now the board.
    sh(
        &repo.remote(),
        &["update-ref", "refs/heads/quecto/board", &h2.head.0],
    );
    assert_eq!(index.rebuild(&h2, &repo.alice), rebuilt(&h2));
    assert_eq!(ids(index.by_status(TaskStatus::Ready)), ["a"]);
}

/// Each remote has an index of its own, its name spelt in characters safe
/// in any file name.
#[test]
fn each_remote_has_an_index_of_its_own() {
    let repo = Repo::new();
    let alice = repo.dir.path().join("alice");
    let url = repo.remote();
    for name in ["upstream", "Up.stream"] {
        sh(&alice, &["remote", "add", name, url.to_str().unwrap()]);
    }
    let named = |remote: &str| {
        let vars: Vec<(std::ffi::OsString, std::ffi::OsString)> = vec![
            ("PATH".into(), std::env::var_os("PATH").unwrap()),
            ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ];
        let identity = Identity {
            name: "Ada".into(),
            email: "ada@example.com".into(),
        };
        let git = Git::new(&alice, GitEnvironment::allowlisted(vars), identity).unwrap();
        let store = GitBoardStore::new(git, remote).unwrap();
        let path = BoardIndex::of(&store).unwrap().path().to_path_buf();
        path.file_name().unwrap().to_string_lossy().into_owned()
    };
    assert_eq!(named("origin"), "project-board-origin.sqlite");
    assert_eq!(named("upstream"), "project-board-upstream.sqlite");
    assert_eq!(named("Up.stream"), "project-board-%55p%2Estream.sqlite");
}

/// A remote that cannot be reached keeps the index as it is.
#[test]
fn an_unreachable_remote_keeps_the_index() {
    let repo = Repo::new();
    let index = BoardIndex::of(&repo.alice).unwrap();
    repo.bootstrapped();
    let board = repo.add("a");
    assert_eq!(
        index.refresh(&repo.alice, &repo.alice),
        Ok(Refreshed::Rebuilt {
            head: board.head.0.clone()
        })
    );
    let alice = repo.dir.path().join("alice");
    let nowhere = repo.dir.path().join("nowhere.git");
    sh(
        &alice,
        &["remote", "set-url", "origin", nowhere.to_str().unwrap()],
    );
    let refreshed = index.refresh(&repo.alice, &repo.alice);
    assert!(
        matches!(
            &refreshed,
            Err(IndexError::Store(BoardStoreError::Unavailable(_)))
        ),
        "{refreshed:?}"
    );
    assert_eq!(index.head(), Ok(Some(board.head.0)));
    assert_eq!(ids(index.by_status(TaskStatus::Ready)), ["a"]);
}
