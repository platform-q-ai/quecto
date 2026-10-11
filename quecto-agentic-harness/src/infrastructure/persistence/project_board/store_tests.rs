use super::*;
use crate::domain::project_board::entities::claim::Identity;
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
        claim: None,
        prs: vec![],
        created: at.clone(),
        updated: at,
    })
    .unwrap()
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
    assert_eq!(store(&repos.alice).read(), Err(StoreError::NoBoard));
    let Ok(Bootstrapped::Created { head }) = store(&repos.alice).bootstrap() else {
        panic!("created")
    };
    assert_eq!(
        store(&repos.bob).bootstrap(),
        Ok(Bootstrapped::Existing { head: head.clone() })
    );
    assert_eq!(
        store(&repos.alice).bootstrap(),
        Ok(Bootstrapped::Existing { head: head.clone() })
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
        Err(StoreError::Clash(_))
    ));
    sh(&repos.alice, &["push", "-q", "origin", "quecto"]);
    assert!(matches!(
        store(&repos.bob).bootstrap(),
        Err(StoreError::Clash(_))
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
    let Ok(Bootstrapped::Created { head }) = alice.bootstrap() else {
        panic!("created")
    };
    let written = alice
        .write(&head, &[task("a"), task("b")], "board: add a and b")
        .unwrap();
    let snapshot = store(&repos.bob).read().unwrap();
    assert_eq!(
        (snapshot.head.as_str(), ids(&snapshot)),
        (written.as_str(), vec!["a", "b"])
    );
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
        format!("Ada <ada@example.com>|Ada|{head}|board: add a and b")
    );
}

#[test]
fn a_write_from_a_stale_base_loses_and_refetches_then_succeeds_on_the_new_head() {
    let repos = Repos::new();
    let (alice, bob) = (store(&repos.alice), store(&repos.bob));
    let Ok(Bootstrapped::Created { head }) = alice.bootstrap() else {
        panic!("created")
    };
    let stale = bob.read().unwrap().head;
    let won = alice.write(&head, &[task("a")], "board: add a").unwrap();
    assert_eq!(
        bob.write(&stale, &[task("b")], "board: add b"),
        Err(StoreError::Lost {
            head: Some(won.clone())
        })
    );
    assert_eq!(
        sh(
            &repos.bob,
            &["rev-parse", "refs/remotes/origin/quecto/board"]
        ),
        won,
        "the loser refetched"
    );
    let fresh = bob.read().unwrap();
    bob.write(&fresh.head, &[task("b")], "board: add b")
        .unwrap();
    assert_eq!(ids(&alice.read().unwrap()), vec!["a", "b"]);
}

#[test]
fn the_checkout_keeps_its_index_head_branches_and_fetch_head() {
    let repos = Repos::new();
    std::fs::write(repos.alice.join("staged.txt"), "staged").unwrap();
    sh(&repos.alice, &["add", "staged.txt"]);
    std::fs::write(repos.alice.join("staged.txt"), "edited").unwrap();
    let before = [
        "--no-optional-locks status --porcelain",
        "rev-parse HEAD",
        "symbolic-ref HEAD",
        "branch --list",
    ]
    .map(|args| sh(&repos.alice, &args.split(' ').collect::<Vec<_>>()));
    let index = std::fs::read(repos.alice.join(".git/index")).unwrap();
    let alice = store(&repos.alice);
    let Ok(Bootstrapped::Created { head }) = alice.bootstrap() else {
        panic!("created")
    };
    alice.write(&head, &[task("a")], "board: add a").unwrap();
    alice.read().unwrap();
    let after = [
        "--no-optional-locks status --porcelain",
        "rev-parse HEAD",
        "symbolic-ref HEAD",
        "branch --list",
    ]
    .map(|args| sh(&repos.alice, &args.split(' ').collect::<Vec<_>>()));
    assert_eq!(after, before);
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

#[test]
fn hooks_never_run() {
    let repos = Repos::new();
    let marker = repos.dir.path().join("hook-ran");
    for hook in [
        "pre-push",
        "pre-commit",
        "post-commit",
        "reference-transaction",
        "pre-auto-gc",
    ] {
        let path = repos.alice.join(".git/hooks").join(hook);
        std::fs::write(
            &path,
            format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker.display()),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let alice = store(&repos.alice);
    let Ok(Bootstrapped::Created { head }) = alice.bootstrap() else {
        panic!("created")
    };
    alice.write(&head, &[task("a")], "board: add a").unwrap();
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
    let Ok(Bootstrapped::Created { head }) = alice.bootstrap() else {
        panic!("created")
    };
    alice.write(&head, &[task("a")], "board: add a").unwrap();
    assert_eq!(ids(&store(&repos.bob).read().unwrap()), vec!["a"]);
    assert_eq!(sh(&elsewhere, &["for-each-ref"]), "");
    assert!(!index.exists());
}

#[test]
fn unreadable_files_are_set_aside_and_the_rest_of_the_board_is_read() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    let Ok(Bootstrapped::Created { head }) = alice.bootstrap() else {
        panic!("created")
    };
    let head = alice
        .write(&head, &[task("good")], "board: add good")
        .unwrap();
    let remote = repos.remote();
    let blob = |bytes: &[u8]| sh_with(&remote, &["hash-object", "-w", "--stdin"], bytes);
    let mut oversize = encode(&task("big")).unwrap();
    oversize.resize(MAX_TASK_FILE_BYTES + 1, b' ');
    let bad = [
        ("broken.json", blob(b"{")),
        ("Upper.json", blob(&encode(&task("upper")).unwrap())),
        ("named.json", blob(&encode(&task("other")).unwrap())),
        ("big.json", blob(&oversize)),
        ("notes.txt", blob(b"notes")),
    ];
    let mut listing = sh(&remote, &["ls-tree", &format!("{head}:tasks")]);
    for (name, oid) in bad {
        listing += &format!("\n100644 blob {oid}\t{name}");
    }
    listing += &format!(
        "\n040000 tree {}\tsub\n",
        sh_with(&remote, &["mktree"], b"")
    );
    let tasks = sh_with(&remote, &["mktree"], listing.as_bytes());
    let readme = sh(&remote, &["rev-parse", &format!("{head}:README.md")]);
    let root = format!("100644 blob {readme}\tREADME.md\n040000 tree {tasks}\ttasks\n");
    let root = sh_with(&remote, &["mktree"], root.as_bytes());
    let commit = sh(
        &remote,
        &["commit-tree", &root, "-p", &head, "-m", "hand edit"],
    );
    sh(&remote, &["update-ref", BOARD_REF, &commit]);

    let snapshot = store(&repos.bob).read().unwrap();
    assert_eq!(ids(&snapshot), vec!["good"]);
    let reasons: BTreeMap<&str, &str> = snapshot
        .quarantined
        .iter()
        .map(|q| (q.path.as_str(), q.reason.as_str()))
        .collect();
    let paths: Vec<&str> = reasons.keys().copied().collect();
    assert_eq!(
        paths,
        [
            "tasks/Upper.json",
            "tasks/big.json",
            "tasks/broken.json",
            "tasks/named.json",
            "tasks/notes.txt",
            "tasks/sub"
        ]
    );
    assert!(reasons["tasks/big.json"].contains("bytes"), "{reasons:?}");
    assert!(
        reasons["tasks/named.json"].contains("\"other\""),
        "{reasons:?}"
    );
    assert!(
        reasons["tasks/broken.json"].contains("malformed"),
        "{reasons:?}"
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
    assert!(matches!(
        GitBoardStore::new(git, "--upload-pack=x"),
        Err(StoreError::Invalid(_))
    ));
    let alice = store(&repos.alice);
    let Ok(Bootstrapped::Created { head }) = alice.bootstrap() else {
        panic!("created")
    };
    for (base, tasks, message) in [
        ("HEAD", vec![task("a")], "ok"),
        (head.as_str(), vec![task("a")], "two\nlines"),
        (head.as_str(), vec![task("a"), task("a")], "ok"),
        (head.as_str(), vec![], "ok"),
    ] {
        assert!(
            matches!(
                alice.write(base, &tasks, message),
                Err(StoreError::Invalid(_))
            ),
            "{base} {message:?}"
        );
    }
}

#[test]
fn a_git_that_hangs_is_stopped_at_its_deadline() {
    let repos = Repos::new();
    let bin = repos.dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    std::fs::write(bin.join("git"), "#!/bin/sh\nexec sleep 30\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(bin.join("git"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut path = bin.into_os_string();
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap());
    let env = GitEnvironment::allowlisted([("PATH".into(), path)]);
    let mut git = Git::new(&repos.alice, env, person("Ada")).unwrap();
    git.network_timeout = std::time::Duration::from_millis(200);
    let started = std::time::Instant::now();
    let read = GitBoardStore::new(git, "origin").unwrap().read();
    assert!(
        matches!(read, Err(StoreError::Git(GitError::TimedOut { .. }))),
        "{read:?}"
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
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
