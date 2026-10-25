use crate::{
    project_board_history_fixture::{git, text},
    project_board_store::task,
};
use quecto::{
    application::project_board::ports::{BoardWrite, ProjectBoardStore},
    domain::project_board::entities::claim::Identity,
    infrastructure::persistence::project_board::{
        git::{Git, GitEnvironment},
        store::{BOARD_REF, GitBoardStore},
    },
};
use std::{
    ffi::OsString,
    path::Path,
    process::{Command, Output},
};
fn cli(home: &Path, args: &[String], caller: Option<&str>) -> Output {
    let mut command = Command::new("timeout");
    command
        .args(["--signal=KILL", "20"])
        .arg(env!("CARGO_BIN_EXE_quecto"))
        .args(args)
        .current_dir(home)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    if let Some(caller) = caller {
        command.env("QUECTO_BOARD_COMMITTER", caller);
    }
    command.output().unwrap()
}
#[test]
fn fence_real_cli_uses_caller_cas_and_repeats_without_configured_identity() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    git(
        root,
        &["init", "--bare", "--initial-branch=main", "remote.git"],
        b"",
    );
    git(root, &["clone", "remote.git", "clone"], b"");
    let repo = root.join("clone");
    let bare = root.join("remote.git");
    let vars: Vec<(OsString, OsString)> = vec![
        ("PATH".into(), std::env::var_os("PATH").unwrap()),
        ("HOME".into(), root.into()),
        ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
    ];
    let store = GitBoardStore::new(
        Git::new(
            &repo,
            GitEnvironment::allowlisted(vars),
            Identity {
                name: "Fixture Writer".into(),
                email: "fixture@example.test".into(),
            },
        )
        .unwrap(),
        "origin",
    )
    .unwrap();
    store.bootstrap().unwrap();
    let captured = store.read().unwrap().head;
    assert!(matches!(
        store.write_tasks(
            &captured,
            &[task("synthetic", "Offline fixture")],
            "fixture: source"
        ),
        Ok(BoardWrite::Written(_))
    ));
    let before = text(&bare, &["rev-parse", BOARD_REF]);
    git(&bare, &["update-ref", "refs/heads/unrelated", &before], b"");
    let args = vec![
        "board-server".into(),
        "fence".into(),
        "--repo".into(),
        repo.to_str().unwrap().into(),
        "--board-id".into(),
        "0123456789abcdef".into(),
        "--url".into(),
        "https://Primary.example.test/board".into(),
        "--url".into(),
        "http://standby.example.test:7979".into(),
    ];
    let result = cli(root, &args, Some("Offline Caller <offline@example.test>"));
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["status"], "published");
    assert_eq!(report["previous"], before);
    let landed = text(&bare, &["rev-parse", BOARD_REF]);
    assert_eq!(report["head"], landed);
    assert_eq!(
        text(&bare, &["rev-list", "--parents", "-1", &landed]),
        format!("{landed} {before}")
    );
    let raw = text(&bare, &["cat-file", "commit", &landed]);
    for field in ["author", "committer"] {
        assert!(raw.contains(&format!("{field} Offline Caller <offline@example.test> ")));
    }
    let objects = text(&repo, &["cat-file", "--batch-all-objects", "--batch-check"]);
    // This clone has no configured identity at any scope. Already must not
    // enter the lazy writer factory or consult fresh configuration.
    let result = cli(root, &args, None);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let repeat: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(
        repeat,
        serde_json::json!({"status":"already", "head":landed})
    );
    assert_eq!(
        text(&repo, &["cat-file", "--batch-all-objects", "--batch-check"]),
        objects
    );
    let mut other = args.clone();
    other[7] = "https://elsewhere.example.test".into();
    let refused = cli(root, &other, None);
    assert_eq!(refused.status.code(), Some(1));
    let error: serde_json::Value = serde_json::from_slice(&refused.stdout).unwrap();
    assert_eq!(error["error"]["store"]["kind"], "moved");
    assert_eq!(error["error"]["store"]["board_id"], "0123456789abcdef");
    assert_eq!(text(&bare, &["rev-parse", BOARD_REF]), landed);
    assert_eq!(text(&bare, &["rev-parse", "refs/heads/unrelated"]), before);
    let mut names: Vec<_> = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["clone", "remote.git"]);
}
#[test]
fn fence_each_bad_binary_argv_has_own_home_xdg_and_current_directory() {
    for args in [
        vec![
            "--repo",
            "missing",
            "--board-id",
            "bad",
            "--url",
            "https://example.test",
        ],
        vec![
            "--repo",
            "missing",
            "--board-id",
            "0123456789abcdef",
            "--url",
            "file:///bad",
        ],
        vec![
            "--repo",
            "missing",
            "--board-id",
            "0123456789abcdef",
            "--url",
            "https://example.test",
            "--state",
            "bad",
        ],
    ] {
        let home = tempfile::tempdir().unwrap();
        let args: Vec<_> = ["board-server", "fence"]
            .into_iter()
            .chain(args)
            .map(str::to_owned)
            .collect();
        let result = cli(home.path(), &args, None);
        assert_eq!(result.status.code(), Some(1));
        let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(report["error"]["kind"], "invalid_arguments");
        assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0);
    }
}
