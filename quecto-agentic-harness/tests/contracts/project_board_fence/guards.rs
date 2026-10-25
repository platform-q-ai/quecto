use super::fixture::A;
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn inventory(root: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
    fn visit(root: &Path, here: &Path, result: &mut Vec<(PathBuf, Option<Vec<u8>>)>) {
        for entry in std::fs::read_dir(here).unwrap() {
            let path = entry.unwrap().path();
            let metadata = std::fs::symlink_metadata(&path).unwrap();
            result.push((
                path.strip_prefix(root).unwrap().to_owned(),
                if metadata.is_file() {
                    Some(std::fs::read(&path).unwrap())
                } else {
                    None
                },
            ));
            if metadata.is_dir() {
                visit(root, &path, result);
            }
        }
    }
    let mut result = Vec::new();
    visit(root, root, &mut result);
    result.sort();
    result
}

fn cli(home: &Path, args: &[&str]) -> Output {
    Command::new("timeout")
        .current_dir(home)
        .args(["--signal=KILL", "20", env!("CARGO_BIN_EXE_quecto")])
        .args(args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("QUECTO_BASE_DIR", home)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap()
}
#[test]
fn r9_fence_help_preserves_delivered_action_usage() {
    let home = tempfile::tempdir().unwrap();
    let output = cli(home.path(), &["--help"]);
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    for usage in [
        "board-server serve",
        "board-server token",
        "board-server import --repo <source> --board-id <16-hex> [--remote <name>]",
        "board-server export",
    ] {
        assert!(help.contains(usage), "preserve {usage}");
    }
    for usage in [
        "board-server backup --to <dir> [--board-id <16-hex>]",
        "board-server fence --repo <clone> --board-id <16-hex> --url <http(s)> [--url ...] [--remote <name>]",
    ] {
        assert!(
            help.contains(usage),
            "positive offline maintenance help missing: {usage}"
        );
    }
}
#[test]
fn r9_fence_allowlist_refuses_before_io_control() {
    for args in [
        vec![
            "board-server",
            "fence",
            "--repo",
            "absent",
            "--board-id",
            A,
            "--url",
            "file:///tmp/never",
        ],
        vec![
            "board-server",
            "fence",
            "--repo",
            "absent",
            "--board-id",
            A,
            "--url",
            "https://example.test",
            "--remote",
            "a",
            "--remote",
            "b",
        ],
    ] {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join("owned-sentinel"),
            b"unchanged scratch state",
        )
        .unwrap();
        let before = inventory(home.path());
        let output = cli(home.path(), &args);
        assert_eq!(output.status.code(), Some(1));
        assert!(!home.path().join("data/quecto/boards").exists());
        assert!(!home.path().join("out").exists());
        assert_eq!(
            inventory(home.path()),
            before,
            "no relative destination, token, config, state, or listener artifacts anywhere in isolated HOME/cwd"
        );
    }
}
