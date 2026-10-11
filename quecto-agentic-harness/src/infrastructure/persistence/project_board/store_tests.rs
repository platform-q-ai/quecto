use super::*;
use crate::domain::project_board::entities::task::TaskFields;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};
use crate::infrastructure::persistence::project_board::git::GitEnvironment;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Test-side git: only PATH and HOME inherited, no global or system config.
fn sh(dir: &Path, args: &[&str]) -> String {
    sh_with(dir, args, b"")
}

fn sh_with(dir: &Path, args: &[&str], input: &[u8]) -> String {
    use std::io::Write;
    let mut child = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", dir)
        .envs([
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ])
        .envs([("GIT_AUTHOR_NAME", "T"), ("GIT_AUTHOR_EMAIL", "t@x.io")])
        .envs([
            ("GIT_COMMITTER_NAME", "T"),
            ("GIT_COMMITTER_EMAIL", "t@x.io"),
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string()
}

fn vars(home: &Path, extra: &[(&str, &Path)]) -> Vec<(OsString, OsString)> {
    let mut vars = vec![
        ("PATH".into(), std::env::var_os("PATH").unwrap()),
        ("HOME".into(), home.into()),
        ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
    ];
    vars.extend(
        extra
            .iter()
            .map(|(key, value)| ((*key).into(), value.as_os_str().to_owned())),
    );
    vars
}

struct Repos {
    dir: tempfile::TempDir,
    alice: PathBuf,
    bob: PathBuf,
}

impl Repos {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sh(root, &["init", "-q", "--bare", "-b", "main", "remote.git"]);
        sh(root, &["clone", "-q", "remote.git", "alice"]);
        sh(
            &root.join("alice"),
            &["commit", "-q", "--allow-empty", "-m", "init"],
        );
        sh(&root.join("alice"), &["push", "-q", "origin", "main"]);
        sh(root, &["clone", "-q", "remote.git", "bob"]);
        Self {
            alice: root.join("alice"),
            bob: root.join("bob"),
            dir,
        }
    }

    fn remote(&self) -> PathBuf {
        self.dir.path().join("remote.git")
    }
}

fn person(name: &str) -> Identity {
    Identity {
        name: name.into(),
        email: format!("{}@example.com", name.to_lowercase()),
    }
}

fn store_with(repo: &Path, vars: Vec<(OsString, OsString)>) -> GitBoardStore {
    let git = Git::new(repo, GitEnvironment::allowlisted(vars), person("Ada")).unwrap();
    GitBoardStore::new(git, "origin").unwrap()
}

fn store(repo: &Path) -> GitBoardStore {
    store_with(repo, vars(repo, &[]))
}

fn task(id: &str) -> Task {
    let at = Timestamp::parse("2026-10-10T08:00:00Z").unwrap();
    Task::new(TaskFields {
        id: Slug::parse(id).unwrap(),
        title: format!("Task {id}"),
        kind: TaskKind::Task,
        description: "Do it.\n".into(),
        status: TaskStatus::Ready,
        parent: None,
        depends_on: vec![],
        plan: vec![],
        items: vec![],
        team: None,
        claim: None,
        runs: vec![],
        reviews: vec![],
        prs: vec![],
        created: at.clone(),
        updated: at,
    })
    .unwrap()
}

fn created(store: &GitBoardStore) -> BoardHead {
    match store.bootstrap() {
        Ok(BoardBootstrap::Created(head)) => head,
        other => panic!("not created: {other:?}"),
    }
}

fn written(write: Result<BoardWrite, BoardStoreError>) -> BoardHead {
    match write {
        Ok(BoardWrite::Written(head)) => head,
        other => panic!("not written: {other:?}"),
    }
}

/// Moves the remote board to a commit of `tasks/` entries `extra` (name,
/// `mode type oid`) beside the ones at the head.
fn hand_edit(remote: &Path, extra: &[(&str, String)]) -> BoardHead {
    let head = sh(remote, &["rev-parse", BOARD_REF]);
    let mut listing = sh(remote, &["ls-tree", &format!("{head}:tasks")]);
    for (name, meta) in extra {
        listing += &format!("\n{meta}\t{name}");
    }
    let tasks = sh_with(remote, &["mktree"], format!("{listing}\n").as_bytes());
    let readme = sh(remote, &["rev-parse", &format!("{head}:README.md")]);
    let root = format!("100644 blob {readme}\tREADME.md\n040000 tree {tasks}\ttasks\n");
    let root = sh_with(remote, &["mktree"], root.as_bytes());
    let commit = sh(
        remote,
        &["commit-tree", &root, "-p", &head, "-m", "hand edit"],
    );
    sh(remote, &["update-ref", BOARD_REF, &commit]);
    BoardHead(commit)
}

fn ids(snapshot: &BoardSnapshot) -> Vec<&str> {
    snapshot
        .tasks
        .iter()
        .map(|task| task.fields().id.as_str())
        .collect()
}

#[test]
fn bootstrap_creates_the_board_once_with_only_its_readme() {
    let repos = Repos::new();
    assert_eq!(store(&repos.alice).read(), Err(BoardStoreError::NoBoard));
    let head = created(&store(&repos.alice));
    assert_eq!(
        store(&repos.bob).bootstrap(),
        Ok(BoardBootstrap::Existing(head.clone()))
    );
    assert_eq!(
        store(&repos.alice).bootstrap(),
        Ok(BoardBootstrap::Existing(head.clone()))
    );
    assert_eq!(
        sh(&repos.remote(), &["ls-tree", "--name-only", BOARD_REF]),
        "README.md"
    );
    let snapshot = store(&repos.bob).read().unwrap();
    assert_eq!(
        (snapshot.head, snapshot.tasks, snapshot.quarantined),
        (head, vec![], vec![])
    );
}

#[test]
fn a_branch_named_quecto_here_or_on_the_remote_blocks_bootstrap() {
    let repos = Repos::new();
    sh(&repos.alice, &["branch", "quecto"]);
    assert!(matches!(
        store(&repos.alice).bootstrap(),
        Err(BoardStoreError::Clash(_))
    ));
    sh(&repos.alice, &["push", "-q", "origin", "quecto"]);
    assert!(matches!(
        store(&repos.bob).bootstrap(),
        Err(BoardStoreError::Clash(_))
    ));
    assert_eq!(
        sh(&repos.remote(), &["for-each-ref", "refs/heads/quecto/"]),
        ""
    );
}

#[test]
fn tasks_and_their_summaries_are_written_in_one_commit_and_read_back() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    let head = created(&alice);
    let moved = written(alice.write(&head, &[task("a"), task("b")], "board: add a and b"));
    let snapshot = store(&repos.bob).read().unwrap();
    assert_eq!((&snapshot.head, ids(&snapshot)), (&moved, vec!["a", "b"]));
    assert_eq!(snapshot.tasks, vec![task("a"), task("b")]);
    let file = |path: &str| {
        sh(
            &repos.remote(),
            &["cat-file", "blob", &format!("{BOARD_REF}:{path}")],
        )
    };
    assert_eq!(
        file("tasks/a.json") + "\n",
        String::from_utf8(encode(&task("a")).unwrap()).unwrap()
    );
    assert_eq!(file("tasks/a.md"), summary(&task("a")).trim_end());
    let log = sh(
        &repos.remote(),
        &["log", "--format=%an <%ae>|%cn|%P|%s", BOARD_REF],
    );
    assert_eq!(
        log.lines().next().unwrap(),
        format!("Ada <ada@example.com>|Ada|{}|board: add a and b", head.0)
    );
    assert_eq!(alice.writer(), person("Ada"));
}

#[test]
fn the_checkout_keeps_its_index_head_branches_and_fetch_head() {
    let repos = Repos::new();
    std::fs::write(repos.alice.join("staged.txt"), "staged").unwrap();
    sh(&repos.alice, &["add", "staged.txt"]);
    std::fs::write(repos.alice.join("staged.txt"), "edited").unwrap();
    let state = || {
        [
            "--no-optional-locks status --porcelain",
            "rev-parse HEAD",
            "symbolic-ref HEAD",
            "branch --list",
            "tag --list",
        ]
        .map(|args| sh(&repos.alice, &args.split(' ').collect::<Vec<_>>()))
    };
    let before = state();
    let index = std::fs::read(repos.alice.join(".git/index")).unwrap();
    let alice = store(&repos.alice);
    let head = created(&alice);
    written(alice.write(&head, &[task("a")], "board: add a"));
    alice.read().unwrap();
    assert_eq!(state(), before);
    assert_eq!(
        std::fs::read(repos.alice.join(".git/index")).unwrap(),
        index
    );
    assert!(!repos.alice.join(".git/FETCH_HEAD").exists());
    assert_eq!(
        std::fs::read_to_string(repos.alice.join("staged.txt")).unwrap(),
        "edited"
    );
}

fn executable(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn hooks_and_fsmonitor_never_run() {
    let repos = Repos::new();
    let marker = repos.dir.path().join("hook-ran");
    let script = format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker.display());
    for hook in [
        "pre-push",
        "pre-commit",
        "post-commit",
        "reference-transaction",
        "pre-auto-gc",
        "fsmonitor-watchman",
    ] {
        executable(&repos.alice.join(".git/hooks").join(hook), &script);
    }
    let monitor = repos.dir.path().join("fsmonitor");
    executable(&monitor, &script);
    sh(
        &repos.alice,
        &["config", "core.fsmonitor", monitor.to_str().unwrap()],
    );
    let alice = store(&repos.alice);
    let head = created(&alice);
    written(alice.write(&head, &[task("a")], "board: add a"));
    alice.read().unwrap();
    assert!(!marker.exists(), "a hook or fsmonitor ran");
}

#[test]
fn inherited_git_dir_index_and_work_tree_are_ignored() {
    let repos = Repos::new();
    let elsewhere = repos.dir.path().join("elsewhere");
    sh(repos.dir.path(), &["init", "-q", "elsewhere"]);
    let index = repos.dir.path().join("stray-index");
    let extra = [
        ("GIT_DIR", elsewhere.join(".git")),
        ("GIT_WORK_TREE", elsewhere.clone()),
        ("GIT_INDEX_FILE", index.clone()),
    ];
    let extra: Vec<(&str, &Path)> = extra
        .iter()
        .map(|(key, value)| (*key, value.as_path()))
        .collect();
    let alice = store_with(&repos.alice, vars(&repos.alice, &extra));
    let head = created(&alice);
    written(alice.write(&head, &[task("a")], "board: add a"));
    assert_eq!(ids(&store(&repos.bob).read().unwrap()), vec!["a"]);
    assert_eq!(sh(&elsewhere, &["for-each-ref"]), "");
    assert!(!index.exists());
}

#[test]
fn only_regular_task_files_are_read_and_the_rest_is_set_aside_by_id() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    let head = created(&alice);
    written(alice.write(&head, &[task("good")], "board: add good"));
    let remote = repos.remote();
    let blob = |bytes: &[u8]| {
        format!(
            "100644 blob {}",
            sh_with(&remote, &["hash-object", "-w", "--stdin"], bytes)
        )
    };
    let mut oversize = encode(&task("big")).unwrap();
    oversize.resize(MAX_TASK_FILE_BYTES + 1, b' ');
    let target = sh_with(
        &remote,
        &["hash-object", "-w", "--stdin"],
        b"../../etc/passwd",
    );
    let commit = sh(&remote, &["rev-parse", BOARD_REF]);
    let empty = sh_with(&remote, &["mktree"], b"");
    hand_edit(
        &remote,
        &[
            ("broken.json", blob(b"{")),
            ("Upper.json", blob(&encode(&task("upper")).unwrap())),
            ("named.json", blob(&encode(&task("other")).unwrap())),
            ("big.json", blob(&oversize)),
            ("notes.txt", blob(b"notes")),
            (
                "exec.json",
                blob(&encode(&task("exec")).unwrap()).replace("100644", "100755"),
            ),
            ("link.json", format!("120000 blob {target}")),
            ("module.json", format!("160000 commit {commit}")),
            ("sub", format!("040000 tree {empty}")),
        ],
    );
    let bob = store(&repos.bob);
    let snapshot = bob.read().unwrap();
    assert_eq!(ids(&snapshot), vec!["good"]);
    let found: BTreeMap<&str, (Option<&str>, &str)> = snapshot
        .quarantined
        .iter()
        .map(|q| {
            (
                q.path.as_str(),
                (q.id.as_ref().map(Slug::as_str), q.reason.as_str()),
            )
        })
        .collect();
    let by_id: Vec<(&str, Option<&str>)> =
        found.iter().map(|(path, (id, _))| (*path, *id)).collect();
    assert_eq!(
        by_id,
        [
            ("tasks/Upper.json", None),
            ("tasks/big.json", Some("big")),
            ("tasks/broken.json", Some("broken")),
            ("tasks/exec.json", Some("exec")),
            ("tasks/link.json", Some("link")),
            ("tasks/module.json", Some("module")),
            ("tasks/named.json", Some("named")),
            ("tasks/notes.txt", None),
            ("tasks/sub", None),
        ]
    );
    assert!(found["tasks/big.json"].1.contains("bytes"), "{found:?}");
    assert!(
        found["tasks/named.json"].1.contains("\"other\""),
        "{found:?}"
    );
    assert!(
        found["tasks/broken.json"].1.contains("malformed"),
        "{found:?}"
    );
    assert!(found["tasks/link.json"].1.contains("regular"), "{found:?}");

    for id in ["broken", "link", "named"] {
        let refused = bob.write(&snapshot.head, &[task(id)], &format!("board: write {id}"));
        assert!(
            matches!(refused, Ok(BoardWrite::Refused(_))),
            "{id}: {refused:?}"
        );
    }
    assert_eq!(sh(&remote, &["rev-parse", BOARD_REF]), snapshot.head.0);
    // A write carries forward only regular task files, never links,
    // modules, subtrees or strays.
    let moved = written(bob.write(&snapshot.head, &[task("fresh")], "board: add fresh"));
    let kept = sh(
        &remote,
        &["ls-tree", "--name-only", &format!("{}:tasks", moved.0)],
    );
    assert_eq!(
        kept.lines().collect::<Vec<_>>(),
        [
            "big.json",
            "broken.json",
            "fresh.json",
            "fresh.md",
            "good.json",
            "good.md",
            "named.json"
        ]
    );
}

#[test]
fn a_board_tree_past_its_entry_bound_is_not_read() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    created(&alice);
    written(alice.write(&alice.read().unwrap().head, &[task("a")], "board: add a"));
    let blob = sh_with(&repos.remote(), &["hash-object", "-w", "--stdin"], b"x");
    let many: Vec<(String, String)> = (0..MAX_TASK_ENTRIES)
        .map(|n| (format!("n{n}.md"), format!("100644 blob {blob}")))
        .collect();
    let many: Vec<(&str, String)> = many
        .iter()
        .map(|(name, meta)| (name.as_str(), meta.clone()))
        .collect();
    hand_edit(&repos.remote(), &many);
    let read = store(&repos.bob).read();
    assert!(
        matches!(&read, Err(BoardStoreError::Unavailable(why)) if why.contains("entries")),
        "{read:?}"
    );
}

#[test]
fn bad_arguments_are_refused_before_git_runs() {
    let repos = Repos::new();
    let git = Git::new(
        &repos.alice,
        GitEnvironment::allowlisted(vars(&repos.alice, &[])),
        person("Ada"),
    )
    .unwrap();
    assert!(GitBoardStore::new(git, "--upload-pack=x").is_err());
    let alice = store(&repos.alice);
    let head = created(&alice);
    for (base, tasks, message) in [
        ("HEAD", vec![task("a")], "ok"),
        ("--all", vec![task("a")], "ok"),
        (&"0".repeat(40), vec![task("a")], "ok"),
        (head.0.as_str(), vec![task("a")], "two\nlines"),
        (head.0.as_str(), vec![task("a"), task("a")], "ok"),
        (head.0.as_str(), vec![], "ok"),
    ] {
        let refused = alice.write(&BoardHead(base.into()), &tasks, message);
        assert!(
            matches!(refused, Err(BoardStoreError::InvalidWrite(_))),
            "{base} {message:?}: {refused:?}"
        );
    }
    assert_eq!(alice.read().unwrap().head, head);
}

#[test]
fn a_git_that_hangs_is_stopped_with_its_whole_process_group() {
    let repos = Repos::new();
    let bin = repos.dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let marker = repos.dir.path().join("grandchild-lived");
    executable(
        &bin.join("git"),
        &format!(
            "#!/bin/sh\n(sleep 1; touch '{}') &\nexec sleep 30\n",
            marker.display()
        ),
    );
    let mut path = bin.into_os_string();
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap());
    let env = GitEnvironment::allowlisted([("PATH".into(), path)]);
    let mut git = Git::new(&repos.alice, env, person("Ada")).unwrap();
    git.network_timeout = std::time::Duration::from_millis(200);
    let started = std::time::Instant::now();
    let read = GitBoardStore::new(git, "origin").unwrap().read();
    assert!(
        matches!(&read, Err(BoardStoreError::Unavailable(why)) if why.contains("TimedOut")),
        "{read:?}"
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert!(!marker.exists(), "the timed-out git's child outlived it");
}

#[test]
fn only_allowlisted_variables_reach_git() {
    let env = GitEnvironment::allowlisted(
        [
            "PATH",
            "HOME",
            "GIT_DIR",
            "GIT_INDEX_FILE",
            "GIT_WORK_TREE",
            "GIT_SSH_COMMAND",
            "AWS_SECRET_ACCESS_KEY",
        ]
        .map(|key| (OsString::from(key), OsString::from("x"))),
    );
    let kept = format!("{env:?}");
    for key in ["PATH", "HOME", "GIT_SSH_COMMAND"] {
        assert!(kept.contains(key), "{key}");
    }
    for key in [
        "GIT_DIR",
        "GIT_INDEX_FILE",
        "GIT_WORK_TREE",
        "AWS_SECRET_ACCESS_KEY",
    ] {
        assert!(!kept.contains(key), "{key}");
    }
}

#[path = "store_push_tests.rs"]
mod push;
