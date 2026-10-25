use super::fixture::{A, B, GitRemoteScalarWatchdog};
use crate::project_board_history_fixture::{git, text};
use crate::project_board_store::task;
use quecto::application::project_board::ports::{fence::*, *};
use quecto::domain::project_board::{
    entities::claim::Identity,
    value_objects::{board_moved::BoardMoved, telemetry::BoardId},
};
use quecto::infrastructure::persistence::project_board::{
    git::{Git, GitEnvironment},
    store::{BOARD_REF, GitBoardStore},
};
use std::{ffi::OsString, path::PathBuf};

struct Remote {
    _dir: tempfile::TempDir,
    bare: PathBuf,
    clones: [PathBuf; 2],
}
impl Remote {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(
            root,
            &["init", "--bare", "--initial-branch=main", "remote.git"],
            b"",
        );
        for name in ["alice", "bob"] {
            git(root, &["clone", "remote.git", name], b"");
        }
        let remote = Self {
            bare: root.join("remote.git"),
            clones: [root.join("alice"), root.join("bob")],
            _dir: dir,
        };
        let store = remote.store(0);
        store.bootstrap().unwrap();
        let head = store.read().unwrap().head;
        assert!(matches!(
            store.write_tasks(
                &head,
                &[task("synthetic", "Scratch board")],
                "board: scratch fixture"
            ),
            Ok(BoardWrite::Written(_))
        ));
        let head = remote.head();
        git(
            &remote.bare,
            &["update-ref", "refs/heads/unrelated", &head.0],
            b"",
        );
        assert_eq!(remote.store(1).read().unwrap().tasks.len(), 1);
        println!(
            "scratch remote captured={} unrelated={} two owned clones",
            head.0,
            remote.unrelated()
        );
        remote
    }
    fn store(&self, n: usize) -> GitBoardStore {
        self.store_with_path(n, std::env::var_os("PATH").unwrap())
    }
    fn store_with_path(&self, n: usize, path: OsString) -> GitBoardStore {
        let vars: Vec<(OsString, OsString)> = vec![
            ("PATH".into(), path),
            ("HOME".into(), self._dir.path().into()),
            (
                "XDG_CONFIG_HOME".into(),
                self._dir.path().join("config").into(),
            ),
            ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ];
        let writer = Identity {
            name: "Explicit Maintenance Caller".into(),
            email: "maintenance@example.com".into(),
        };
        GitBoardStore::new(
            Git::new(&self.clones[n], GitEnvironment::allowlisted(vars), writer).unwrap(),
            "origin",
        )
        .unwrap()
    }
    fn install_root(&self, additions: Vec<(&str, String)>) -> BoardHead {
        let before = self.head();
        let mut rows = std::collections::BTreeMap::new();
        for row in text(&self.bare, &["ls-tree", &before.0]).lines() {
            let (value, name) = row.split_once('\t').unwrap();
            rows.insert(name.to_owned(), value.to_owned());
        }
        for (name, value) in additions {
            rows.insert(name.to_owned(), value);
        }
        let listing: String = rows
            .into_iter()
            .map(|(name, value)| format!("{value}\t{name}\n"))
            .collect();
        let tree = String::from_utf8(git(&self.bare, &["mktree"], listing.as_bytes()))
            .unwrap()
            .trim()
            .to_owned();
        let commit = text(
            &self.bare,
            &[
                "commit-tree",
                &tree,
                "-p",
                &before.0,
                "-m",
                "fixture: recognized root and archive",
            ],
        );
        git(
            &self.bare,
            &["update-ref", BOARD_REF, &commit, &before.0],
            b"",
        );
        self.head()
    }
    fn rich_root(&self) {
        let empty = String::from_utf8(git(&self.bare, &["mktree"], b""))
            .unwrap()
            .trim()
            .to_owned();
        let blob = String::from_utf8(git(
            &self.bare,
            &["hash-object", "-w", "--stdin"],
            b"synthetic archived bytes",
        ))
        .unwrap()
        .trim()
        .to_owned();
        let archive = String::from_utf8(git(
            &self.bare,
            &["mktree"],
            format!("100644 blob {blob}\tlegacy.raw\n").as_bytes(),
        ))
        .unwrap()
        .trim()
        .to_owned();
        let mut additions: Vec<_> = ["plans", "runs", "changes", "reviews", "discussions"]
            .into_iter()
            .map(|name| (name, format!("040000 tree {empty}")))
            .collect();
        additions.push(("archive", format!("040000 tree {archive}")));
        additions.push(("stray.txt", format!("100644 blob {blob}")));
        self.install_root(additions);
        assert_eq!(self.store(1).read().unwrap().tasks.len(), 1);
    }
    fn head(&self) -> BoardHead {
        BoardHead(text(&self.bare, &["rev-parse", BOARD_REF]))
    }
    fn unrelated(&self) -> String {
        text(&self.bare, &["rev-parse", "refs/heads/unrelated"])
    }
    fn objects(&self) -> String {
        text(
            &self.clones[0],
            &["cat-file", "--batch-all-objects", "--batch-check"],
        )
    }
    // Fixture construction only: existing pure builder plus owned bare update-ref,
    // never the publication method under test. Makes Already independently reachable.
    fn seed_fence(&self) -> BoardHead {
        use quecto::infrastructure::persistence::project_board::store::Fence;
        let store = self.store(0);
        let captured = store.read().unwrap().head;
        let Fence::Tree(tree) = store.fence_tree(&captured.0, &moved()).unwrap() else {
            panic!("fixture starts unfenced")
        };
        let commit = text(
            &self.clones[0],
            &[
                "commit-tree",
                &tree,
                "-p",
                &captured.0,
                "-m",
                "fixture: pre-existing fence",
            ],
        );
        git(
            &self.clones[0],
            &["push", "origin", &format!("{commit}:{BOARD_REF}")],
            b"",
        );
        let head = self.head();
        assert_eq!(head.0, commit);
        assert!(
            matches!(self.store(1).read(), Err(BoardStoreError::Moved(found)) if found == moved())
        );
        head
    }
    fn publish(&self, captured: &BoardHead, moved: &BoardMoved) -> Result<FenceReport, FenceError> {
        let store = self.store(0);
        let port: &dyn BoardFence = &store;
        let result = port.publish_fence(captured, moved);
        println!(
            "operational BoardFence::publish_fence reached captured={} result={result:?}",
            captured.0
        );
        result
    }
}
fn destination(board: &str, urls: &[&str]) -> BoardMoved {
    BoardMoved::new(
        BoardId::of_hex(board).unwrap(),
        urls.iter().map(|u| (*u).into()).collect(),
    )
    .unwrap()
}
fn moved() -> BoardMoved {
    destination(
        A,
        &[
            "https://primary.example.test/board",
            "http://standby.example.test:7979",
        ],
    )
}
use crate::board_fence::published;

#[test]
fn r6_fence_publishes_one_parented_commit() {
    let _deadline = GitRemoteScalarWatchdog::new();
    let r = Remote::new();
    r.rich_root();
    let before = r.head();
    let unrelated = r.unrelated();
    let old_raw = git(&r.bare, &["cat-file", "commit", &before.0], b"");
    let old_tree = text(&r.bare, &["ls-tree", "-r", &before.0]);
    let head = published(r.publish(&before, &moved()), &before);
    assert_eq!(r.head(), head);
    assert_eq!(
        git(&r.bare, &["show", &format!("{}:format/board", head.0)], b""),
        b"quecto-board 4\n"
    );
    for folder in [
        "tasks",
        "plans",
        "runs",
        "changes",
        "reviews",
        "discussions",
        "archive",
    ] {
        assert_eq!(
            text(&r.bare, &["rev-parse", &format!("{}:{folder}", head.0)]),
            text(&r.bare, &["rev-parse", &format!("{}:{folder}", before.0)]),
            "new fence carries recognized {folder} exactly"
        );
    }
    let root_names = text(&r.bare, &["ls-tree", "--name-only", &head.0]);
    assert_eq!(
        root_names
            .lines()
            .collect::<std::collections::BTreeSet<_>>(),
        [
            "README.md",
            "archive",
            "changes",
            "discussions",
            "format",
            "plans",
            "reviews",
            "runs",
            "tasks"
        ]
        .into_iter()
        .collect(),
        "strays dropped, all recognized roots retained"
    );

    assert_eq!(
        text(&r.bare, &["rev-list", "--parents", "-1", &head.0]),
        format!("{} {}", head.0, before.0),
        "sole captured parent, no unrelated parent"
    );
    let raw = String::from_utf8(git(&r.bare, &["cat-file", "commit", &head.0], b"")).unwrap();
    for field in ["author", "committer"] {
        assert!(raw.contains(&format!(
            "{field} Explicit Maintenance Caller <maintenance@example.com> "
        )));
    }
    assert_eq!(
        git(&r.bare, &["show", &format!("{}:format/MOVED", head.0)], b""),
        moved().file()
    );
    assert_eq!(
        git(&r.bare, &["cat-file", "commit", &before.0], b""),
        old_raw
    );
    assert_eq!(text(&r.bare, &["ls-tree", "-r", &before.0]), old_tree);
    assert_eq!(r.unrelated(), unrelated);
    assert!(matches!(r.store(1).read(), Err(BoardStoreError::Moved(found)) if found == moved()));
}

#[test]
fn r7_fence_exact_repeat_noops_elsewhere_refuses() {
    let _deadline = GitRemoteScalarWatchdog::new();
    let r = Remote::new();
    let head = r.seed_fence();
    let objects = r.objects();
    assert_eq!(
        r.publish(&head, &moved()),
        Ok(FenceReport::Already { head: head.clone() }),
        "exact repeat must return Already without fresh identity/objects/push"
    );
    assert_eq!(
        r.objects(),
        objects,
        "Already creates no new tree, blob or commit objects"
    );
    assert_eq!(r.head(), head);
    for other in [
        destination(
            A,
            &[
                "http://standby.example.test:7979",
                "https://primary.example.test/board",
            ],
        ),
        destination(A, &["https://elsewhere.example.test"]),
        destination(
            B,
            &[
                "https://primary.example.test/board",
                "http://standby.example.test:7979",
            ],
        ),
    ] {
        assert!(
            matches!(r.publish(&head, &other), Err(FenceError::Store(BoardStoreError::Moved(found))) if found == moved()),
            "ordered destination or board change must refuse with existing typed Moved"
        );
        assert_eq!(r.head(), head);
        assert_eq!(r.objects(), objects);
    }
}

#[test]
fn r8_fence_cas_preserves_competing_winner() {
    let _deadline = GitRemoteScalarWatchdog::new();
    let r = Remote::new();
    let captured = r.head();
    // Explicit captured-head port is the deterministic capture/push barrier.
    let other = r.store(1);
    let observed = other.read().unwrap().head;
    assert_eq!(observed, captured);
    let written = other
        .write_tasks(
            &observed,
            &[task("winner", "Competing writer")],
            "board: concurrent winner",
        )
        .unwrap();
    assert!(matches!(written, BoardWrite::Written(_)));
    let winner = r.head();
    assert_ne!(winner, captured);
    println!(
        "CAS barrier captured={} winner={} before fence call",
        captured.0, winner.0
    );
    let unrelated = r.unrelated();
    let result = r.publish(&captured, &moved());
    assert_eq!(r.head(), winner, "stale fence must never replace winner");
    assert_eq!(r.unrelated(), unrelated);
    assert!(
        matches!(result, Err(FenceError::Conflict)),
        "typed CAS conflict required, not missing capability: {result:?}"
    );
    assert!(
        r.store(1)
            .read()
            .unwrap()
            .tasks
            .iter()
            .any(|t| t.fields().id.as_str() == "winner")
    );
}

#[test]
fn r8_fence_transport_refusal_preserves_remote() {
    let _deadline = GitRemoteScalarWatchdog::new();
    let r = Remote::new();
    let before = r.head();
    let unrelated = r.unrelated();
    let hook = r.bare.join("hooks/pre-receive");
    std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(hook, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let result = r.publish(&before, &moved());
    assert_eq!(r.head(), before);
    assert_eq!(r.unrelated(), unrelated);
    assert!(
        matches!(result, Err(FenceError::Refused(_))),
        "transport refusal remains distinct: {result:?}"
    );
}

#[test]
fn r9_after_publication_ordinary_read_and_write_return_typed_moved() {
    let _deadline = GitRemoteScalarWatchdog::new();
    let r = Remote::new();
    let before = r.head();
    let head = published(r.publish(&before, &moved()), &before);
    let store = r.store(1);
    assert!(matches!(store.read(), Err(BoardStoreError::Moved(found)) if found == moved()));
    assert!(
        matches!(store.write_tasks(&before, &[task("refused", "Never written")], "board: refused after move"), Err(BoardStoreError::Moved(found)) if found == moved())
    );
    assert_eq!(r.head(), head);
}

#[test]
fn r7_elsewhere_is_typed_moved_without_remote_change() {
    let _deadline = GitRemoteScalarWatchdog::new();
    let r = Remote::new();
    let head = r.seed_fence();
    let objects = r.objects();
    let unrelated = r.unrelated();
    let other = destination(B, &["https://elsewhere.example.test"]);
    let result = r.publish(&head, &other);
    assert_eq!(r.head(), head);
    assert_eq!(r.unrelated(), unrelated);
    assert_eq!(r.objects(), objects);
    assert_eq!(
        result,
        Err(FenceError::Store(BoardStoreError::Moved(moved())))
    );
}

#[test]
fn r9_existing_builder_moved_read_write_control() {
    let _deadline = GitRemoteScalarWatchdog::new();
    let r = Remote::new();
    let before = r.head();
    let head = r.seed_fence();
    let store = r.store(1);
    assert!(matches!(store.read(), Err(BoardStoreError::Moved(found)) if found == moved()));
    assert!(
        matches!(store.write_tasks(&before, &[task("no-write", "Refused")], "board: refused control"), Err(BoardStoreError::Moved(found)) if found == moved())
    );
    assert_eq!(r.head(), head);
}

#[test]
fn r7_missing_branch_never_bootstraps() {
    let _deadline = GitRemoteScalarWatchdog::new();
    let r = Remote::new();
    let captured = r.head();
    git(&r.bare, &["update-ref", "-d", BOARD_REF], b"");
    let unrelated = r.unrelated();
    let result = r.publish(&captured, &moved());
    assert!(text(&r.bare, &["for-each-ref", BOARD_REF]).is_empty());
    assert_eq!(r.unrelated(), unrelated);
    assert_eq!(result, Err(FenceError::Store(BoardStoreError::NoBoard)));
}

#[cfg(unix)]
fn transport(r: &Remote, mode: &str, winner: Option<&BoardHead>) -> (GitBoardStore, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    assert!(["lost-answer", "uncertain", "race"].contains(&mode));
    let bins = r._dir.path().join("transport-bin");
    std::fs::create_dir(&bins).unwrap();
    let path = std::env::var_os("PATH").unwrap();
    let real = std::env::split_paths(&path)
        .map(|p| p.join("git"))
        .find(|p| p.is_file())
        .unwrap();
    let real = std::fs::canonicalize(real).unwrap();
    let quote = |p: &std::path::Path| format!("'{}'", p.to_str().unwrap().replace('\'', "'\"'\"'"));
    let marker = r._dir.path().join("push-completed");
    let log = r._dir.path().join("push-count");
    std::fs::write(&log, b"").unwrap();
    let captured = r.head();
    let race = match winner {
        Some(winner) => format!(
            "timeout --signal=KILL 10 {} --git-dir={} update-ref {} {} {} || exit 97\n",
            quote(&real),
            quote(&r.bare),
            BOARD_REF,
            winner.0,
            captured.0
        ),
        None => String::new(),
    };
    let refuse_judge = if mode == "uncertain" {
        format!(
            "case \" $* \" in *\" ls-remote \"*) if test -f {}; then exit 96; fi;; esac\n",
            quote(&marker)
        )
    } else {
        String::new()
    };
    let push = if mode == "race" {
        format!(
            "{race}exec timeout --signal=KILL 10 {} \"$@\"\n",
            quote(&real)
        )
    } else {
        format!(
            "timeout --signal=KILL 10 {} \"$@\" >/dev/null 2>&1\ncode=$?\nif test \"$code\" -eq 0; then : > {}; exit 1; fi\nexit \"$code\"\n",
            quote(&real),
            quote(&marker)
        )
    };
    let script = format!(
        "#!/bin/sh\nulimit -c 0\n{refuse_judge}case \" $* \" in\n *\" push \"*) printf 'push\\n' >> {}\n{push};;\nesac\nexec timeout --signal=KILL 10 {} \"$@\"\n",
        quote(&log),
        quote(&real)
    );
    let wrapper = bins.join("git");
    std::fs::write(&wrapper, script).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let joined =
        std::env::join_paths(std::iter::once(bins).chain(std::env::split_paths(&path))).unwrap();
    (r.store_with_path(0, joined), log)
}

#[cfg(unix)]
#[test]
fn r8_lost_push_answer_is_reconciled_against_real_remote() {
    let _deadline = GitRemoteScalarWatchdog::new();
    let r = Remote::new();
    let before = r.head();
    let unrelated = r.unrelated();
    let (store, log) = transport(&r, "lost-answer", None);
    let port: &dyn BoardFence = &store;
    let result = port.publish_fence(&before, &moved());
    println!("operational lost-answer publication reached: {result:?}");
    let head = published(result, &before);
    assert_eq!(r.head(), head);
    assert_eq!(r.unrelated(), unrelated);
    assert_eq!(
        std::fs::read_to_string(log).unwrap(),
        "push\n",
        "no retry/refence after uncertain transport status"
    );
    assert_eq!(
        text(&r.bare, &["rev-list", "--parents", "-1", &head.0]),
        format!("{} {}", head.0, before.0)
    );
}

#[cfg(unix)]
#[test]
fn r8_lost_answer_and_unreadable_judge_reports_uncertain_without_retry() {
    let _deadline = GitRemoteScalarWatchdog::new();
    let r = Remote::new();
    let before = r.head();
    let unrelated = r.unrelated();
    let (store, log) = transport(&r, "uncertain", None);
    let port: &dyn BoardFence = &store;
    let result = port.publish_fence(&before, &moved());
    println!("operational uncertain publication reached: {result:?}");
    assert!(
        matches!(result, Err(FenceError::Uncertain(_))),
        "lost answer and unavailable reconciliation remain explicitly uncertain: {result:?}"
    );
    let landed = r.head();
    assert_ne!(
        landed, before,
        "fixture really committed before dropping its answer"
    );
    assert_eq!(r.unrelated(), unrelated);
    assert_eq!(
        std::fs::read_to_string(log).unwrap(),
        "push\n",
        "no blind retry"
    );
    assert!(matches!(r.store(1).read(), Err(BoardStoreError::Moved(found)) if found == moved()));
}

#[cfg(unix)]
#[test]
fn r8_competing_update_after_capture_before_push_uses_exact_lease() {
    let _deadline = GitRemoteScalarWatchdog::new();
    let r = Remote::new();
    let before = r.head();
    let unrelated = r.unrelated();
    let tree = text(&r.bare, &["rev-parse", &format!("{}^{{tree}}", before.0)]);
    let winner = BoardHead(text(
        &r.bare,
        &[
            "commit-tree",
            &tree,
            "-p",
            &before.0,
            "-m",
            "fixture: concurrent winner at push barrier",
        ],
    ));
    assert_ne!(winner, before);
    let (store, log) = transport(&r, "race", Some(&winner));
    // Adapter sees a valid captured current head; wrapper moves the remote only
    // when the actual private push runs, after capture/fence construction.
    assert_eq!(r.head(), before);
    println!(
        "exact-lease barrier captured={} prepared-winner={}",
        before.0, winner.0
    );
    let port: &dyn BoardFence = &store;
    let result = port.publish_fence(&before, &moved());
    println!("operational push-boundary CAS reached: {result:?}");
    assert_eq!(
        result,
        Err(FenceError::Conflict),
        "actual stale lease must report conflict"
    );
    assert_eq!(r.head(), winner);
    assert_eq!(r.unrelated(), unrelated);
    assert_eq!(std::fs::read_to_string(log).unwrap(), "push\n");
}

#[path = "fence_guards.rs"]
mod guards;
