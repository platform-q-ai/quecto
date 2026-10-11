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
    // Written on its own thread: git may fill its output before it has
    // read all its input.
    let mut stdin = child.stdin.take().unwrap();
    let input = input.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let output = child.wait_with_output().unwrap();
    writer.join().unwrap().unwrap();
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
fn branches_that_would_shadow_the_board_block_bootstrap_ignoring_case() {
    for (local, remote) in [
        (Some("Quecto"), None),
        (None, Some("QUECTO/board")),
        (None, Some("quecto/Board")),
        (None, Some("Quecto/other")),
        (None, Some("quecto/board/x")),
        (Some("quecto/Board/x"), None),
    ] {
        let repos = Repos::new();
        if let Some(branch) = local {
            sh(&repos.alice, &["branch", branch]);
        }
        if let Some(branch) = remote {
            sh(
                &repos.bob,
                &["push", "-q", "origin", &format!("HEAD:refs/heads/{branch}")],
            );
        }
        let bootstrap = store(&repos.alice).bootstrap();
        assert!(
            matches!(bootstrap, Err(BoardStoreError::Clash(_))),
            "{local:?} {remote:?}: {bootstrap:?}"
        );
    }
    let names = [
        "refs/heads/quecto/other",
        "refs/heads/quecto/board",
        "refs/heads/main",
        "refs/heads/quectoboard",
        "refs/heads/quecto/boardx",
    ];
    assert_eq!(
        GitBoardStore::clashing(names.into_iter(), "refs/heads/"),
        Vec::<String>::new()
    );
}

#[test]
fn a_quecto_board_branch_that_is_not_a_board_blocks_bootstrap() {
    let repos = Repos::new();
    sh(
        &repos.alice,
        &["push", "-q", "origin", "HEAD:refs/heads/quecto/board"],
    );
    let bootstrap = store(&repos.bob).bootstrap();
    assert!(
        matches!(&bootstrap, Err(BoardStoreError::Clash(why)) if why.contains("not a board")),
        "{bootstrap:?}"
    );
}

/// What bootstrap calls a board is what read and write take as one: a
/// README, a `tasks` tree, and stray files at the root (each write drops
/// them). A branch with directories of its own is someone's code, never a
/// board: reading it, writing it and bootstrapping over it all clash.
#[test]
fn bootstrap_read_and_write_agree_on_what_a_board_is() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    let head = created(&alice);
    written(alice.write(&head, &[task("a")], "board: add a"));
    let remote = repos.remote();
    let tip = sh(&remote, &["rev-parse", BOARD_REF]);
    let stray = sh_with(&remote, &["hash-object", "-w", "--stdin"], b"notes");
    let listing = sh(&remote, &["ls-tree", &tip]);
    let root = sh_with(
        &remote,
        &["mktree"],
        format!("{listing}\n100644 blob {stray}\tnotes.txt\n").as_bytes(),
    );
    let commit = sh(&remote, &["commit-tree", &root, "-p", &tip, "-m", "stray"]);
    sh(&remote, &["update-ref", BOARD_REF, &commit]);
    let bob = store(&repos.bob);
    assert_eq!(
        bob.bootstrap(),
        Ok(BoardBootstrap::Existing(BoardHead(commit.clone())))
    );
    let snapshot = bob.read().unwrap();
    assert_eq!(ids(&snapshot), ["a"]);
    let moved = written(bob.write(&snapshot.head, &[task("b")], "board: add b"));
    assert_eq!(
        sh(&remote, &["ls-tree", "--name-only", &moved.0]),
        "README.md\ntasks"
    );

    // Someone's code pushed over the board.
    std::fs::write(repos.alice.join("README.md"), "code").unwrap();
    std::fs::create_dir(repos.alice.join("src")).unwrap();
    std::fs::write(repos.alice.join("src/lib.rs"), "").unwrap();
    sh(&repos.alice, &["add", "README.md", "src"]);
    sh(&repos.alice, &["commit", "-q", "-m", "code"]);
    sh(
        &repos.alice,
        &["push", "-q", "-f", "origin", "HEAD:refs/heads/quecto/board"],
    );
    let code = sh(&repos.alice, &["rev-parse", "HEAD"]);
    let bob = store(&repos.bob);
    let clashes = |result: &str| result.starts_with("Err(Clash(");
    assert!(clashes(&format!("{:?}", bob.bootstrap())));
    assert!(clashes(&format!("{:?}", bob.read())));
    let write = bob.write(&BoardHead(code.clone()), &[task("c")], "board: add c");
    assert!(clashes(&format!("{write:?}")), "{write:?}");
    assert_eq!(sh(&remote, &["rev-parse", BOARD_REF]), code);
}

/// A bootstrap that loses the race to a branch that is not a board clashes;
/// it never answers that branch as the existing board.
#[test]
fn a_bootstrap_lost_to_a_branch_that_is_not_a_board_clashes() {
    let repos = Repos::new();
    let main = sh(&repos.alice, &["rev-parse", "HEAD"]);
    let wrapper = repos.dir.path().join("receive-pack");
    executable(
        &wrapper,
        &format!(
            "#!/bin/sh
git --git-dir='{}' update-ref {BOARD_REF} {main}
exec git-receive-pack \"$@\"\n",
            repos.remote().display()
        ),
    );
    sh(
        &repos.alice,
        &[
            "config",
            "remote.origin.receivepack",
            wrapper.to_str().unwrap(),
        ],
    );
    let bootstrap = store(&repos.alice).bootstrap();
    assert!(
        matches!(&bootstrap, Err(BoardStoreError::Clash(why)) if why.contains("not a board")),
        "{bootstrap:?}"
    );
}

/// A write starts only from the board's head or one of its ancestors:
/// another local commit, even one on top of the board, is no board head.
#[test]
fn a_write_from_a_commit_that_is_not_on_the_board_is_invalid() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    let head = created(&alice);
    let main = sh(&repos.alice, &["rev-parse", "HEAD"]);
    let unpushed = sh(
        &repos.alice,
        &[
            "commit-tree",
            &format!("{}^{{tree}}", head.0),
            "-p",
            &head.0,
            "-m",
            "unpushed",
        ],
    );
    for base in [main, unpushed] {
        let write = alice.write(&BoardHead(base.clone()), &[task("a")], "board: add a");
        assert!(
            matches!(&write, Err(BoardStoreError::InvalidWrite(_))),
            "{base}: {write:?}"
        );
    }
    assert_eq!(sh(&repos.remote(), &["rev-parse", BOARD_REF]), head.0);
}

/// The remote is a configured remote of the repository, never a path or
/// URL that happens to be spelled like a name.
#[test]
fn a_remote_that_is_not_configured_is_refused() {
    let repos = Repos::new();
    for remote in ["nowhere", "remote.git"] {
        let git = Git::new(
            &repos.alice,
            GitEnvironment::allowlisted(vars(&repos.alice, &[])),
            person("Ada"),
        )
        .unwrap();
        let built = GitBoardStore::new(git, remote).map(|_| ());
        assert!(
            matches!(&built, Err(BoardStoreError::Unavailable(why)) if why.contains(remote)),
            "{remote}: {built:?}"
        );
    }
}

/// In a partial clone (`--filter=blob:none`) the board's blobs come in one
/// fetch, never one lazy fetch per blob, and the store reads and writes as
/// in a full clone.
#[test]
fn a_partial_clone_fetches_the_board_once_and_reads_and_writes_it() {
    let repos = Repos::new();
    sh(
        &repos.remote(),
        &["config", "uploadpack.allowFilter", "true"],
    );
    let alice = store(&repos.alice);
    let head = created(&alice);
    let many: Vec<Task> = (0..8).map(|n| task(&format!("t{n}"))).collect();
    written(alice.write(&head, &many, "board: add eight"));
    let url = format!("file://{}", repos.remote().display());
    sh(
        repos.dir.path(),
        &["clone", "-q", "--filter=blob:none", &url, "carol"],
    );
    let carol = repos.dir.path().join("carol");
    let count = repos.dir.path().join("upload-packs");
    let wrapper = repos.dir.path().join("upload-pack");
    executable(
        &wrapper,
        &format!(
            "#!/bin/sh
echo x >> '{}'
exec git-upload-pack \"$@\"\n",
            count.display()
        ),
    );
    sh(
        &carol,
        &[
            "config",
            "remote.origin.uploadpack",
            wrapper.to_str().unwrap(),
        ],
    );
    let fetches = || {
        std::fs::read_to_string(&count)
            .map(|text| text.lines().count())
            .unwrap_or(0)
    };
    let partial = store(&carol);
    let snapshot = partial.read().unwrap();
    assert_eq!(snapshot.tasks, many);
    assert!(
        fetches() <= 3,
        "{} upload-pack runs to read the board",
        fetches()
    );
    let moved = written(partial.write(&snapshot.head, &[task("carol")], "board: add carol"));
    let read = store(&repos.bob).read().unwrap();
    assert_eq!(read.head, moved);
    assert_eq!(read.tasks.len(), 9);
}

#[test]
fn every_write_regenerates_the_summaries_and_readme_and_drops_strays() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    let head = created(&alice);
    written(alice.write(&head, &[task("good")], "board: add good"));
    let remote = repos.remote();
    // As long as the right summary, so only its bytes tell them apart.
    let mut hostile = b"<img src=https://track.example/x>".to_vec();
    hostile.resize(summary(&task("good")).len(), b' ');
    let hostile = sh_with(&remote, &["hash-object", "-w", "--stdin"], &hostile);
    let head = sh(&remote, &["rev-parse", BOARD_REF]);
    let tasks = sh(&remote, &["ls-tree", &format!("{head}:tasks")]).replace(
        &sh(&remote, &["rev-parse", &format!("{head}:tasks/good.md")]),
        &hostile,
    );
    let tasks = format!("{tasks}\n100644 blob {hostile}\torphan.md\n");
    let tasks = sh_with(&remote, &["mktree"], tasks.as_bytes());
    let root = format!("100644 blob {hostile}\tREADME.md\n040000 tree {tasks}\ttasks\n");
    let root = sh_with(&remote, &["mktree"], root.as_bytes());
    let commit = sh(
        &remote,
        &["commit-tree", &root, "-p", &head, "-m", "hand edit"],
    );
    sh(&remote, &["update-ref", BOARD_REF, &commit]);
    let bob = store(&repos.bob);
    let moved = written(bob.write(
        &bob.read().unwrap().head,
        &[task("other")],
        "board: add other",
    ));
    let file = |path: &str| {
        sh(
            &remote,
            &["cat-file", "blob", &format!("{}:{path}", moved.0)],
        )
    };
    assert_eq!(file("tasks/good.md"), summary(&task("good")).trim_end());
    assert_eq!(file("README.md") + "\n", String::from_utf8_lossy(README));
    let names = sh(
        &remote,
        &["ls-tree", "--name-only", &format!("{}:tasks", moved.0)],
    );
    assert_eq!(
        names.lines().collect::<Vec<_>>(),
        ["good.json", "good.md", "other.json", "other.md"]
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
fn hooks_never_run() {
    let repos = Repos::new();
    let marker = repos.dir.path().join("hook-ran");
    let script = format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker.display());
    for hook in [
        "pre-push",
        "pre-commit",
        "post-commit",
        "reference-transaction",
        "pre-auto-gc",
    ] {
        executable(&repos.alice.join(".git/hooks").join(hook), &script);
    }
    let alice = store(&repos.alice);
    let head = created(&alice);
    written(alice.write(&head, &[task("a")], "board: add a"));
    alice.read().unwrap();
    assert!(!marker.exists(), "a hook ran");
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

#[path = "store_hostile_tests.rs"]
mod hostile;
#[path = "store_push_tests.rs"]
mod push;
#[path = "git_runner_tests.rs"]
mod runner;
