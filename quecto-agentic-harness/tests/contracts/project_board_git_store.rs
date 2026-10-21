//! The project board store's contract, run against the git store: two
//! clones of one bare remote, each its own client.
use super::project_board_store::{
    Clients, check_bootstrap, check_bootstrap_race, check_carried_set_aside, check_clear_set_aside,
    check_committed_as_writer, check_compare_and_swap, check_invalid_writes, check_lock_races,
    check_quarantine, check_refused, check_unchanged_write, check_write_after_lost_and_refused,
    check_write_and_read, check_write_before_bootstrap, check_writer,
};
use quecto::domain::project_board::entities::claim::Identity;
use quecto::infrastructure::persistence::project_board::git::{Git, GitEnvironment};
use quecto::infrastructure::persistence::project_board::store::{BOARD_REF, GitBoardStore};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Test-side git: only PATH and HOME, no global or system config.
pub fn sh(dir: &Path, args: &[&str], input: &[u8]) -> String {
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
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
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

/// A bare remote and two clones of it.
pub struct Remote {
    _dir: tempfile::TempDir,
    bare: PathBuf,
    clones: [PathBuf; 2],
}

/// The hand edits below rebuild the board's root from its README and
/// `tasks` alone: unmarked roots, as a board written before the format
/// marker had (the store reads them as such, and its next write adds the
/// marker).
impl Remote {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sh(
            root,
            &["init", "-q", "--bare", "-b", "main", "remote.git"],
            b"",
        );
        sh(root, &["clone", "-q", "remote.git", "alice"], b"");
        sh(root, &["clone", "-q", "remote.git", "bob"], b"");
        Self {
            bare: root.join("remote.git"),
            clones: [root.join("alice"), root.join("bob")],
            _dir: dir,
        }
    }

    pub fn client(&self, clone: usize, name: &str) -> Box<GitBoardStore> {
        let repo = &self.clones[clone];
        let vars: [(OsString, OsString); 4] = [
            ("PATH".into(), std::env::var_os("PATH").unwrap()),
            ("HOME".into(), repo.into()),
            ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ];
        let writer = Identity {
            name: name.into(),
            email: format!("{}@example.com", name.to_lowercase()),
        };
        let git = Git::new(repo, GitEnvironment::allowlisted(vars), writer).unwrap();
        Box::new(GitBoardStore::new(git, "origin").unwrap())
    }

    /// Clone `clone`'s working directory.
    pub fn clone_dir(&self, clone: usize) -> &Path {
        &self.clones[clone]
    }

    pub fn clients(&self) -> Clients {
        (self.client(0, "Alice"), self.client(1, "Bob"))
    }

    /// The remote declines its next push, as a server-side hook would.
    fn refuse_next_push(&self) {
        use std::os::unix::fs::PermissionsExt;
        let hook = self.bare.join("hooks/pre-receive");
        std::fs::write(
            &hook,
            "#!/bin/sh\nrm -- \"$0\"\necho 'board is frozen' >&2\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Someone pushes a board whose `tasks/` lists `b.md` twice (git's
    /// `mktree` writes such a tree; only `fsck` refuses it).
    pub fn duplicate_b(&self) {
        let bare = &self.bare;
        let head = sh(bare, &["rev-parse", BOARD_REF], b"");
        let other = sh(bare, &["hash-object", "-w", "--stdin"], b"{}");
        let listing = sh(bare, &["ls-tree", &format!("{head}:tasks")], b"");
        let listing = format!("{listing}\n100644 blob {other}\tb.md\n");
        let tasks = sh(bare, &["mktree"], listing.as_bytes());
        let readme = sh(bare, &["rev-parse", &format!("{head}:README.md")], b"");
        let root = format!("100644 blob {readme}\tREADME.md\n040000 tree {tasks}\ttasks\n");
        let root = sh(bare, &["mktree"], root.as_bytes());
        let commit = sh(
            bare,
            &["commit-tree", &root, "-p", &head, "-m", "hand edit"],
            b"",
        );
        sh(bare, &["update-ref", BOARD_REF, &commit], b"");
    }

    /// Someone pushes a stray `tasks/notes.txt` (not a task file).
    pub fn add_stray(&self) {
        let bare = &self.bare;
        let head = sh(bare, &["rev-parse", BOARD_REF], b"");
        let notes = sh(bare, &["hash-object", "-w", "--stdin"], b"notes");
        let listing = sh(bare, &["ls-tree", &format!("{head}:tasks")], b"");
        let listing = format!("{listing}\n100644 blob {notes}\tnotes.txt\n");
        let tasks = sh(bare, &["mktree"], listing.as_bytes());
        let readme = sh(bare, &["rev-parse", &format!("{head}:README.md")], b"");
        let root = format!("100644 blob {readme}\tREADME.md\n040000 tree {tasks}\ttasks\n");
        let root = sh(bare, &["mktree"], root.as_bytes());
        let commit = sh(
            bare,
            &["commit-tree", &root, "-p", &head, "-m", "stray"],
            b"",
        );
        sh(bare, &["update-ref", BOARD_REF, &commit], b"");
    }

    /// Someone pushes `bytes` as `<folder>/<name>`, in place of a file of
    /// that name or beside the folder's others, keeping the rest of the
    /// board's root as it is.
    pub fn put(&self, folder: &str, name: &str, bytes: &[u8]) {
        let bare = &self.bare;
        let head = sh(bare, &["rev-parse", BOARD_REF], b"");
        let blob = sh(bare, &["hash-object", "-w", "--stdin"], bytes);
        // `listing` with the row named `name` replaced by `row`, or added.
        let put = |listing: &str, name: &str, row: String| -> String {
            let mut rows: Vec<String> = listing
                .lines()
                .filter(|line| !line.ends_with(&format!("\t{name}")))
                .map(str::to_string)
                .collect();
            rows.push(row);
            rows.join("\n") + "\n"
        };
        let root = sh(bare, &["ls-tree", &head], b"");
        let held = root
            .lines()
            .any(|line| line.ends_with(&format!("\t{folder}")));
        let listing = match held {
            true => sh(bare, &["ls-tree", &format!("{head}:{folder}")], b""),
            false => String::new(),
        };
        let rows = put(&listing, name, format!("100644 blob {blob}\t{name}"));
        let tree = sh(bare, &["mktree"], rows.as_bytes());
        let root = put(&root, folder, format!("040000 tree {tree}\t{folder}"));
        let root = sh(bare, &["mktree"], root.as_bytes());
        let commit = sh(
            bare,
            &["commit-tree", &root, "-p", &head, "-m", "hand edit"],
            b"",
        );
        sh(bare, &["update-ref", BOARD_REF, &commit], b"");
    }

    /// The object `path` names on the remote's board, as git spells it.
    pub fn object(&self, path: &str) -> String {
        sh(
            &self.bare,
            &["rev-parse", &format!("{BOARD_REF}:{path}")],
            b"",
        )
    }

    /// Someone pushes a `tasks/b.md` the store cannot read.
    pub fn spoil_b(&self) {
        let bare = &self.bare;
        let head = sh(bare, &["rev-parse", BOARD_REF], b"");
        let broken = sh(bare, &["hash-object", "-w", "--stdin"], b"{");
        let listing = sh(bare, &["ls-tree", &format!("{head}:tasks")], b"");
        let listing: String = listing
            .lines()
            .map(|row| match row.ends_with("\tb.md") {
                true => format!("100644 blob {broken}\tb.md\n"),
                false => format!("{row}\n"),
            })
            .collect();
        let tasks = sh(bare, &["mktree"], listing.as_bytes());
        let readme = sh(bare, &["rev-parse", &format!("{head}:README.md")], b"");
        let root = format!("100644 blob {readme}\tREADME.md\n040000 tree {tasks}\ttasks\n");
        let root = sh(bare, &["mktree"], root.as_bytes());
        let commit = sh(
            bare,
            &["commit-tree", &root, "-p", &head, "-m", "hand edit"],
            b"",
        );
        sh(bare, &["update-ref", BOARD_REF, &commit], b"");
    }
}

#[test]
fn the_git_store_bootstraps_once() {
    check_bootstrap(Remote::new().clients());
}

#[test]
fn the_git_store_counts_lock_races_only_when_it_meets_one() {
    check_lock_races(Remote::new().clients());
}

#[test]
fn the_git_store_writes_and_reads_back() {
    check_write_and_read(Remote::new().clients());
}

#[test]
fn the_git_store_writes_only_at_the_head() {
    check_compare_and_swap(Remote::new().clients());
}

#[test]
fn the_git_store_refuses_invalid_writes() {
    check_invalid_writes(Remote::new().clients());
}

#[test]
fn the_git_store_refuses_without_moving() {
    let remote = Remote::new();
    check_refused(remote.clients(), || remote.refuse_next_push());
}

#[test]
fn the_git_store_clears_a_file_set_aside() {
    let remote = Remote::new();
    check_clear_set_aside(remote.clients(), || remote.spoil_b());
}

#[test]
fn the_git_store_sets_unreadable_files_aside() {
    let remote = Remote::new();
    check_quarantine(remote.clients(), || remote.spoil_b());
}

#[test]
fn the_git_store_moves_its_head_on_an_unchanged_write() {
    check_unchanged_write(Remote::new().clients());
}

#[test]
fn the_git_store_writes_again_after_a_lost_or_refused_write() {
    let remote = Remote::new();
    check_write_after_lost_and_refused(remote.clients(), || remote.refuse_next_push());
}

#[test]
fn the_git_store_takes_no_write_before_it_is_bootstrapped() {
    check_write_before_bootstrap(Remote::new().clients());
}

#[test]
fn the_git_store_bootstraps_once_when_two_race() {
    check_bootstrap_race(Remote::new().clients());
}

#[test]
fn the_git_store_writes_as_a_valid_committer() {
    check_writer(Remote::new().clients());
}

#[test]
fn the_git_store_commits_as_its_writer() {
    let remote = Remote::new();
    let reader = remote.client(0, "Reader");
    check_committed_as_writer(remote.clients(), |head| {
        reader
            .committed_by(&head.0)
            .expect("the commit is on the remote")
    });
}

#[test]
fn the_git_store_carries_a_task_file_set_aside_and_drops_a_stray() {
    let remote = Remote::new();
    check_carried_set_aside(
        remote.clients(),
        || remote.spoil_b(),
        || remote.add_stray(),
        "tasks/notes.txt",
    );
}

/// #2481 M4b: a plan is proposed and its task items edited through the
/// git store, each a commit of its plan alone: the task's file keeps its
/// object, and the approach the edit left out reads back byte for byte
/// from another clone.
#[test]
fn the_git_store_takes_a_plan_proposed_then_patched() {
    use quecto::application::project_board::dto::{
        NewBoardTask, NewBoardTaskStatus, PlanEdit, PlanProposal,
    };
    use quecto::application::project_board::ports::ProjectBoardStore;
    use quecto::application::project_board::use_cases::{
        BootstrapProjectBoard, CreateBoardTask, EditPlan, ProposePlan,
    };
    use quecto::domain::project_board::entities::plan::PlanItem;
    use quecto::domain::project_board::value_objects::slug::Slug;
    use quecto::domain::project_board::value_objects::vocabulary::{PlanState, TaskKind};
    use quecto::infrastructure::time::board_clock::SystemBoardClock;
    use std::sync::Arc;

    let remote = Remote::new();
    let alice: Box<dyn ProjectBoardStore> = remote.client(0, "Alice");
    let store: Arc<dyn ProjectBoardStore> = Arc::from(alice);
    let clock = Arc::new(SystemBoardClock::default());
    let id = Slug::parse("board-store").unwrap();
    BootstrapProjectBoard::new(Arc::clone(&store))
        .execute()
        .unwrap();
    CreateBoardTask::new(Arc::clone(&store), clock.clone())
        .unwrap()
        .execute(&NewBoardTask {
            id: id.clone(),
            title: "Board store".into(),
            kind: TaskKind::Task,
            description: String::new(),
            status: NewBoardTaskStatus::Ready,
            parent: None,
            depends_on: vec![],
        })
        .unwrap();
    let task_file = remote.object("tasks/board-store.md");
    let item = |name: &str| PlanItem {
        id: Slug::parse(name).unwrap(),
        title: format!("Item {name}"),
        criteria: vec![format!("{name} proved")],
        depends_on: vec![],
    };
    let proposed = ProposePlan::new(Arc::clone(&store), clock.clone())
        .unwrap()
        .execute(&PlanProposal {
            task: id.clone(),
            approach: "Spike the store.\r\n\n```\n  keep  spaced\n```\n".into(),
            items: vec![item("schema")],
            team: None,
        })
        .unwrap();
    let approach = "Spike the store.\n\n```\n  keep  spaced\n```";
    assert_eq!(
        proposed.fields().approach,
        approach,
        "normalised as it enters"
    );
    let edited = EditPlan::new(Arc::clone(&store), clock)
        .unwrap()
        .execute(&PlanEdit {
            task: id.clone(),
            version: 1,
            approach: None,
            items: Some(vec![item("schema"), item("api")]),
            team: None,
            expected_updated: Some(proposed.fields().updated.clone()),
        })
        .unwrap();
    assert_eq!(remote.object("tasks/board-store.md"), task_file);
    let bob = remote.client(1, "Bob");
    let board = bob.read().unwrap();
    assert_eq!(board.plans, vec![edited.clone()]);
    let read = board.plans[0].fields();
    assert_eq!(read.approach.as_bytes(), approach.as_bytes());
    assert_eq!((read.version, read.state), (1, PlanState::Draft));
    assert_eq!(read.items, vec![item("schema"), item("api")]);
    assert_eq!(bob.committed_by(&board.head.0).unwrap().name, "Alice");
}

/// #2481 M4c: an acceptance through the git store is one commit of three
/// files, task first (the change shape the commit allowlist takes): the
/// task naming the version, the plan accepted and the plan superseded,
/// read back from another clone as written.
#[test]
fn the_git_store_takes_an_acceptance_of_three_files_in_one_commit() {
    use quecto::application::project_board::dto::{
        NewBoardTask, NewBoardTaskStatus, PlanAcceptance, PlanProposal,
    };
    use quecto::application::project_board::ports::ProjectBoardStore;
    use quecto::application::project_board::use_cases::{
        AcceptPlan, BootstrapProjectBoard, CreateBoardTask, ProposePlan,
    };
    use quecto::domain::project_board::value_objects::slug::Slug;
    use quecto::domain::project_board::value_objects::vocabulary::{PlanState, TaskKind};
    use quecto::infrastructure::time::board_clock::SystemBoardClock;
    use std::sync::Arc;

    let remote = Remote::new();
    let alice: Box<dyn ProjectBoardStore> = remote.client(0, "Alice");
    let store: Arc<dyn ProjectBoardStore> = Arc::from(alice);
    let clock = Arc::new(SystemBoardClock::default());
    let id = Slug::parse("board-store").unwrap();
    BootstrapProjectBoard::new(Arc::clone(&store))
        .execute()
        .unwrap();
    CreateBoardTask::new(Arc::clone(&store), clock.clone())
        .unwrap()
        .execute(&NewBoardTask {
            id: id.clone(),
            title: "Board store".into(),
            kind: TaskKind::Task,
            description: String::new(),
            status: NewBoardTaskStatus::Ready,
            parent: None,
            depends_on: vec![],
        })
        .unwrap();
    let propose = ProposePlan::new(Arc::clone(&store), clock.clone()).unwrap();
    let accept = AcceptPlan::new(Arc::clone(&store), clock).unwrap();
    let proposal = |approach: &str| PlanProposal {
        task: id.clone(),
        approach: approach.into(),
        items: vec![],
        team: None,
    };
    let first = propose.execute(&proposal("First.")).unwrap();
    let acceptance = |plan: &quecto::domain::project_board::entities::plan::Plan,
                      seen: Option<u32>| PlanAcceptance {
        task: id.clone(),
        version: plan.fields().version,
        expected_updated: plan.fields().updated.clone(),
        expected_accepted: seen,
    };
    accept.execute(&acceptance(&first, None)).unwrap();
    let second = propose.execute(&proposal("Second.")).unwrap();
    let before = remote.object("plans/board-store.1.md");
    let accepted = accept.execute(&acceptance(&second, Some(1))).unwrap();
    assert_ne!(
        remote.object("plans/board-store.1.md"),
        before,
        "superseded"
    );
    let bob = remote.client(1, "Bob");
    let board = bob.read().unwrap();
    assert_eq!(board.tasks, vec![accepted.task.clone()]);
    assert_eq!(board.tasks[0].fields().accepted_plan, Some(2));
    let states: Vec<(u32, PlanState)> = board
        .plans
        .iter()
        .map(|plan| (plan.fields().version, plan.fields().state))
        .collect();
    assert_eq!(
        states,
        [(1, PlanState::Superseded), (2, PlanState::Accepted)]
    );
    assert_eq!(board.plans[1], accepted.plan);
    assert_eq!(Some(&board.plans[0]), accepted.superseded.as_ref());
    // One commit wrote all three files, and no other.
    let args = [
        "diff-tree",
        "--no-commit-id",
        "--name-only",
        "-r",
        BOARD_REF,
    ];
    let changed = sh(&remote.bare, &args, b"");
    assert_eq!(
        changed.lines().collect::<Vec<_>>(),
        [
            "plans/board-store.1.md",
            "plans/board-store.2.md",
            "tasks/board-store.md"
        ]
    );
}

/// A started task's run folded back through the git store (#2481 S4): the
/// run alone lands in `runs/`, its addresses linked and its secrets never
/// written, and the board reads it back as folded.
#[test]
fn the_git_store_takes_a_run_folded_back_alone_with_no_secret_in_its_file() {
    use quecto::application::project_board::dto::FoldedBack;
    use quecto::application::project_board::fold_back::{
        SwarmEnd, SwarmEvidence, SwarmItemState, SwarmItemView, SwarmRunView,
    };
    use quecto::application::project_board::ports::{BoardArtifact, BoardWrite};
    use quecto::application::project_board::use_cases::FoldBackRun;
    use quecto::domain::project_board::entities::plan::{Plan, PlanFields, PlanItem};
    use quecto::domain::project_board::entities::task::{Task, TaskFields};
    use quecto::domain::project_board::services::starts::start;
    use quecto::domain::project_board::value_objects::slug::Slug;
    use quecto::domain::project_board::value_objects::timestamp::Timestamp;
    use quecto::domain::project_board::value_objects::vocabulary::{
        ItemState, PlanState, RunOutcome, TaskKind, TaskStatus,
    };
    use quecto::infrastructure::persistence::project_board::text_cells::MarkdownTextCells;
    use quecto::infrastructure::time::board_clock::SystemBoardClock;
    use std::sync::Arc;

    const TOKEN: &str = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    let remote = Remote::new();
    let (alice, bob) = remote.clients();
    alice.bootstrap().unwrap();
    let slug = |text: &str| Slug::parse(text).unwrap();
    let at = Timestamp::parse("2026-10-10T08:00:00Z").unwrap();
    let ada = Identity {
        name: "Alice".into(),
        email: "alice@example.com".into(),
    };
    let ready = Task::new(TaskFields {
        id: slug("board-store"),
        title: "Board store".into(),
        kind: TaskKind::Task,
        description: String::new(),
        status: TaskStatus::Ready,
        parent: None,
        depends_on: vec![],
        accepted_plan: Some(1),
        claim: None,
        prs: vec![],
        github_issue: None,
        resolution: None,
        superseded_by: vec![],
        references: vec![],
        created: at.clone(),
        updated: at.clone(),
    })
    .unwrap();
    let plan = Plan::new(PlanFields {
        task: slug("board-store"),
        version: 1,
        state: PlanState::Accepted,
        approach: "Build it.".into(),
        items: ["api", "docs"]
            .map(|id| PlanItem {
                id: slug(id),
                title: format!("Build the {id}"),
                criteria: vec![],
                depends_on: vec![],
            })
            .to_vec(),
        team: None,
        proposed_by: ada.clone(),
        edited_by: None,
        accepted: Some((ada.clone(), at.clone())),
        created: at.clone(),
        updated: at.clone(),
    })
    .unwrap();
    let started = Timestamp::parse("2026-10-10T09:00:00Z").unwrap();
    let (task, run) = start(&ready, &[], &[], Some(&plan), &ada, &started)
        .unwrap()
        .into_writes();
    let head = alice.read().unwrap().head;
    let artifacts = [
        BoardArtifact::Task(task.clone()),
        BoardArtifact::Plan(plan),
        BoardArtifact::Run(run),
    ];
    let written = alice.write(&head, &artifacts, "board: board-store started");
    assert!(matches!(written, Ok(BoardWrite::Written(_))), "{written:?}");

    let view = SwarmRunView {
        end: SwarmEnd::Succeeded,
        items: vec![SwarmItemView {
            title: "api: Build the api".into(),
            state: SwarmItemState::Completed,
            owner: Some("worker-1".into()),
            evidence: [
                (
                    format!("https://github.com/o/r/pull/7?token={TOKEN}"),
                    "4f2a9c1",
                ),
                (TOKEN.to_string(), "4f2a9c1"),
                ("costs $5".to_string(), "4f2a9c1"),
                ("postgres://app:S3cr#t@db:5432/app".to_string(), "4f2a9c1"),
                ("https://h.example/p#sig=ab".to_string(), "4f2a9c1"),
                (
                    "`https://acct.blob.core.windows.net/c/b?sv=2024&sig=AbC%2Bs3cr3t`".into(),
                    "4f2a9c1",
                ),
            ]
            .map(|(artifact, revision)| SwarmEvidence {
                artifact,
                revision: revision.into(),
            })
            .to_vec(),
            stood_before_death: None,
        }],
        observed_tokens: 52_000,
    };
    let use_case = FoldBackRun::new(
        Arc::from(bob),
        Arc::new(SystemBoardClock::default()),
        Arc::new(MarkdownTextCells),
    )
    .unwrap();
    let folded = use_case.execute(&slug("board-store"), 1, &view).unwrap();
    assert!(matches!(folded, FoldedBack::Folded(_)), "{folded:?}");

    let board = alice.read().unwrap();
    assert_eq!(board.tasks, [task], "the task is left as it was");
    let [run] = board.runs.as_slice() else {
        panic!("one run: {:?}", board.runs);
    };
    let fields = run.fields();
    assert_eq!(
        (fields.outcome, fields.cost),
        (Some(RunOutcome::Succeeded), Some(52_000))
    );
    let states: Vec<ItemState> = fields.items.iter().map(|item| item.state).collect();
    assert_eq!(states, [ItemState::Done, ItemState::Todo]);
    let file = sh(
        &remote.bare,
        &["show", &format!("{BOARD_REF}:runs/board-store.1.md")],
        b"",
    );
    assert!(!file.contains(TOKEN) && !file.contains("$5"), "{file}");
    assert!(!file.contains("sig=") && !file.contains("sv="), "{file}");
    assert!(!file.contains("S3cr") && !file.contains("app:"), "{file}");
    // A fragment that is no anchor puts its line in doubt: dropped whole.
    assert!(!file.contains("h.example/p"), "{file}");
    assert!(
        file.contains("`https://acct.blob.core.windows.net/c/b` at 4f2a9c1"),
        "{file}"
    );
    // A line naming a credential word (`token`) is dropped whole.
    assert!(!file.contains("pull/7"), "{file}");
    assert!(file.contains("[REDACTED] at 4f2a9c1"), "{file}");
    let log = sh(&remote.bare, &["log", "-1", "--format=%s", BOARD_REF], b"");
    assert_eq!(log, "board: run 1 of board-store folded back, succeeded");
}
