//! The project board store's contract, run against the git store: two
//! clones of one bare remote, each its own client.
use super::project_board_store::{
    Clients, check_bootstrap, check_compare_and_swap, check_invalid_writes, check_quarantine,
    check_refused, check_write_and_read,
};
use quecto::domain::project_board::entities::claim::Identity;
use quecto::infrastructure::persistence::project_board::git::{Git, GitEnvironment};
use quecto::infrastructure::persistence::project_board::store::{BOARD_REF, GitBoardStore};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Test-side git: only PATH and HOME, no global or system config.
fn sh(dir: &Path, args: &[&str], input: &[u8]) -> String {
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

struct Remote {
    _dir: tempfile::TempDir,
    bare: PathBuf,
    clones: [PathBuf; 2],
}

impl Remote {
    fn new() -> Self {
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

    fn client(&self, clone: usize, name: &str) -> Box<GitBoardStore> {
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

    fn clients(&self) -> Clients {
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

    /// Someone pushes a `tasks/b.json` the store cannot read.
    fn spoil_b(&self) {
        let bare = &self.bare;
        let head = sh(bare, &["rev-parse", BOARD_REF], b"");
        let broken = sh(bare, &["hash-object", "-w", "--stdin"], b"{");
        let listing = sh(bare, &["ls-tree", &format!("{head}:tasks")], b"");
        let listing: String = listing
            .lines()
            .map(|row| match row.ends_with("\tb.json") {
                true => format!("100644 blob {broken}\tb.json\n"),
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
fn the_git_store_sets_unreadable_files_aside() {
    let remote = Remote::new();
    check_quarantine(remote.clients(), || remote.spoil_b());
}

#[test]
fn the_git_store_writer_is_its_committer() {
    use quecto::application::project_board::ports::ProjectBoardStore;
    let remote = Remote::new();
    assert_eq!(remote.client(0, "Alice").writer().name, "Alice");
}
