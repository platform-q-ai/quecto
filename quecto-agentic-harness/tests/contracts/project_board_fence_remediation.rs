//! Permanent PR2751 M1/M2 regressions; only owned local remotes and child env.
use quecto::{
    application::project_board::ports::ProjectBoardStore,
    domain::project_board::{
        entities::claim::Identity,
        value_objects::{board_moved::BoardMoved, telemetry::BoardId},
    },
    infrastructure::persistence::project_board::{
        git::{Git, GitEnvironment},
        store::{BOARD_REF, GitBoardStore},
    },
};
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::mpsc,
    time::Duration,
};
const BOARD: &str = "0123456789abcdef";
const URLS: [&str; 2] = [
    "https://Primary.example.test/board?token=SYNTHETIC_PR2751_SECRET",
    "http://standby.example.test:7979/board",
];
struct Watchdog(
    Option<mpsc::Sender<()>>,
    Option<std::thread::JoinHandle<()>>,
);
impl Watchdog {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            if matches!(
                rx.recv_timeout(Duration::from_secs(45)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                // SAFETY: _exit takes only a scalar status and terminates without memory preconditions.
                unsafe { libc::_exit(124) }
            }
        });
        Self(Some(tx), Some(thread))
    }
}
impl Drop for Watchdog {
    fn drop(&mut self) {
        self.0.take().unwrap().send(()).ok();
        self.1.take().unwrap().join().unwrap();
    }
}
fn command(home: &Path, program: &str, seconds: &str) -> Command {
    let mut command = Command::new("timeout");
    command
        .args(["--signal=KILL", seconds, program])
        .current_dir(home)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0");
    command
}
fn git(repo: &Path, args: &[&str]) -> String {
    let output = command(repo, "git", "10").args(args).output().unwrap();
    assert!(output.status.success(), "fixture Git failed: {args:?}");
    String::from_utf8(output.stdout).unwrap().trim_end().into()
}
struct Fixture {
    _watchdog: Watchdog,
    dir: tempfile::TempDir,
    repo: PathBuf,
    bare: PathBuf,
    args: Vec<String>,
}
impl Fixture {
    fn new() -> Self {
        let watchdog = Watchdog::new();
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        git(
            home,
            &["init", "--bare", "--initial-branch=main", "remote.git"],
        );
        git(home, &["clone", "remote.git", "clone"]);
        let repo = home.join("clone");
        let bare = home.join("remote.git");
        let vars: Vec<(OsString, OsString)> = vec![
            ("PATH".into(), std::env::var_os("PATH").unwrap()),
            ("HOME".into(), home.into()),
            ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ];
        let store = GitBoardStore::new(
            Git::new(
                &repo,
                GitEnvironment::allowlisted(vars),
                Identity {
                    name: "Synthetic Fixture".into(),
                    email: "fixture@example.test".into(),
                },
            )
            .unwrap(),
            "origin",
        )
        .unwrap();
        store.bootstrap().unwrap();
        let before = git(&bare, &["rev-parse", BOARD_REF]);
        git(&bare, &["update-ref", "refs/heads/unrelated", &before]);
        let args = [
            "board-server",
            "fence",
            "--repo",
            repo.to_str().unwrap(),
            "--board-id",
            BOARD,
            "--url",
            URLS[0],
            "--url",
            URLS[1],
        ]
        .map(str::to_owned)
        .to_vec();
        Self {
            _watchdog: watchdog,
            dir,
            repo,
            bare,
            args,
        }
    }
    fn cli(&self, args: &[String], identity: Option<&OsStr>) -> Output {
        let mut child = command(self.dir.path(), env!("CARGO_BIN_EXE_quecto"), "20");
        child.args(args);
        if let Some(identity) = identity {
            child.env("QUECTO_BOARD_COMMITTER", identity);
        }
        child.output().unwrap()
    }
    fn publish(&self) -> String {
        let result = self.cli(
            &self.args,
            Some(OsStr::new("Synthetic Caller <caller@example.test>")),
        );
        assert!(result.status.success(), "valid publication failed");
        let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(json["status"], "published");
        git(&self.bare, &["rev-parse", BOARD_REF])
    }
    fn state(&self) -> (String, String, String) {
        (
            git(&self.bare, &["show-ref"]),
            git(
                &self.repo,
                &["cat-file", "--batch-all-objects", "--batch-check"],
            ),
            git(
                &self.bare,
                &["cat-file", "--batch-all-objects", "--batch-check"],
            ),
        )
    }
    fn already(&self, identity: Option<&OsStr>) {
        let head = self.publish();
        let before = self.state();
        let result = self.cli(&self.args, identity);
        assert_eq!(
            result.status.code(),
            Some(0),
            "exact Already must not validate a fresh writer identity"
        );
        assert!(result.stderr.is_empty());
        let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(json, serde_json::json!({"status":"already", "head":head}));
        assert_eq!(
            self.state(),
            before,
            "Already must not create objects or change refs"
        );
    }
}
#[test]
fn remediation_already_without_identity_is_legitimate_control() {
    Fixture::new().already(None);
}
#[test]
fn remediation_already_ignores_malformed_explicit_identity() {
    Fixture::new().already(Some(OsStr::new("malformed-synthetic")));
}
#[cfg(unix)]
#[test]
fn remediation_already_ignores_non_utf8_explicit_identity() {
    use std::os::unix::ffi::OsStrExt;
    Fixture::new().already(Some(OsStr::from_bytes(b"synthetic-\xff")));
}
#[test]
fn remediation_real_publish_refuses_invalid_explicit_identity_control() {
    let fixture = Fixture::new();
    let mut identities = vec![OsString::from("malformed-synthetic")];
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        identities.push(OsString::from_vec(b"synthetic-\xff".to_vec()));
    }
    let before = fixture.state();
    let local_refs = git(&fixture.repo, &["show-ref"]);
    let captured = git(&fixture.bare, &["rev-parse", BOARD_REF]);
    for identity in identities {
        let result = fixture.cli(&fixture.args, Some(&identity));
        assert_eq!(result.status.code(), Some(1));
        let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert!(["unavailable", "store"].contains(&json["error"]["kind"].as_str().unwrap()));
        if json["error"]["kind"] == "store" {
            assert_eq!(json["error"]["store"]["kind"], "unavailable");
        }
        assert!(String::from_utf8_lossy(&result.stderr).contains("QUECTO_BOARD_COMMITTER"));
        // A refused writer may follow local fence-tree construction, but cannot publish.
        let after = fixture.state();
        assert_eq!(
            after.0, before.0,
            "invalid Publish must not change remote refs"
        );
        assert_eq!(
            after.2, before.2,
            "invalid Publish must not send remote objects"
        );
        assert_eq!(git(&fixture.repo, &["show-ref"]), local_refs);
        assert_eq!(git(&fixture.bare, &["rev-parse", BOARD_REF]), captured);
    }
}
#[test]
fn remediation_typed_moved_redacts_json_and_stderr_without_changing_internal_urls() {
    let fixture = Fixture::new();
    let head = fixture.publish();
    // The sole builder persists the original ordered DTO, not a redacted destination.
    let raw = git(&fixture.bare, &["show", &format!("{head}:format/MOVED")]);
    let expected = BoardMoved::new(
        BoardId::of_hex(BOARD).unwrap(),
        URLS.map(str::to_owned).to_vec(),
    )
    .unwrap();
    assert_eq!(raw, String::from_utf8(expected.file()).unwrap().trim_end());
    assert_eq!(
        BoardMoved::parse(format!("{raw}\n").as_bytes()).unwrap(),
        expected
    );
    assert_eq!(expected.urls(), URLS);
    let before = fixture.state();
    let mut other = fixture.args.clone();
    other[5] = "fedcba9876543210".into();
    let result = fixture.cli(&other, None);
    assert_eq!(result.status.code(), Some(1));
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert!(stderr.contains("[REDACTED]"));
    assert!(stderr.contains("standby.example.test"));
    assert_eq!(stderr.contains("SYNTHETIC_PR2751_SECRET"), false);
    let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(json["error"]["kind"], "store");
    assert_eq!(json["error"]["store"]["kind"], "moved");
    assert_eq!(json["error"]["store"]["board_id"], BOARD);
    assert_eq!(
        json["error"]["store"]["urls"],
        serde_json::json!(["https://Primary.example.test/board?[REDACTED]", URLS[1]])
    );
    assert_eq!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains("SYNTHETIC_PR2751_SECRET"),
        false
    );
    assert_eq!(fixture.state(), before);
}
#[test]
fn remediation_forbidden_userinfo_keeps_generic_redaction_control() {
    let fixture = Fixture::new();
    let before = fixture.state();
    let mut bad = fixture.args.clone();
    bad[7] = "https://synthetic:FAKESECRET@example.test/".into();
    let result = fixture.cli(&bad, None);
    assert_eq!(result.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(json["error"]["kind"], "invalid_arguments");
    for stream in [&result.stdout, &result.stderr] {
        let text = String::from_utf8_lossy(stream);
        assert!(text.contains("https://***@example.test/"));
        assert_eq!(text.contains("FAKESECRET"), false);
    }
    assert_eq!(fixture.state(), before);
}
