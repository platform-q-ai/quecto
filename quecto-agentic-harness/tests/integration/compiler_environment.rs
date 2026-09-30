//! Real offline compiler-boundary and secret-free entry regressions.
#![cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn sanitizer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".quecto/containers/standard/build-environment.sh")
}
fn bounded(seconds: &str) -> Command {
    let mut command = Command::new("timeout");
    command.env_clear().args(["--signal=KILL", seconds]);
    command.env("PATH", "/usr/bin:/bin");
    command
}
fn clean_command(args: &[&str]) -> Output {
    bounded("5s").arg(sanitizer()).args(args).output().unwrap()
}
#[test]
fn entry_environment_has_only_explicit_names() {
    let output = bounded("5s")
        .arg(sanitizer())
        .arg("/usr/bin/env")
        .env("CARGO_HOME", "/tmp/cargo-fixture")
        .env("SCCACHE_DIR", "/tmp/sccache-fixture")
        .env("SCCACHE_CACHE_SIZE", "2G")
        .env("RUSTUP_HOME", "/opt/rustup")
        .env("CARGO_PKG_NAME", "forged")
        .env("OUT_DIR", "forged")
        .env("CARGO_REGISTRY_TOKEN", "fixture")
        .env("CARGO_REGISTRIES_FIXTURE_TOKEN", "fixture")
        .env("ANTHROPIC_API_KEY", "fixture")
        .env("OPENAI_API_KEY", "fixture")
        .env("GH_TOKEN", "fixture")
        .env("UNRECOGNISED_PROVIDER_CREDENTIAL", "fixture")
        .env("QUECTO_SWARM_BOOTSTRAP", "fixture")
        .output()
        .unwrap();
    assert!(output.status.success());
    let names: Vec<_> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| line.split_once('=').unwrap().0.to_owned())
        .collect();
    assert!(
        names.iter().all(|name| [
            "PATH",
            "CARGO_HOME",
            "SCCACHE_DIR",
            "SCCACHE_CACHE_SIZE",
            "RUSTUP_HOME",
            "TERM"
        ]
        .contains(&name.as_str())),
        "{names:?}"
    );
    assert!(
        ["CARGO_HOME", "SCCACHE_DIR", "SCCACHE_CACHE_SIZE"]
            .iter()
            .all(|name| names.iter().any(|actual| actual == name))
    );
}
#[test]
fn compiler_protocol_allows_metadata_without_cargo_credentials() {
    let root = tempfile::tempdir().unwrap();
    for tool in ["rustc", "rustdoc", "sccache"] {
        let executable = root.path().join(tool);
        fs::write(
            &executable,
            "#!/bin/bash -p\n/usr/bin/tr '\\0' '\\n' < /proc/$$/environ\n",
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let output = bounded("5s")
            .arg(sanitizer())
            .arg(executable)
            .env("CARGO_PKG_NAME", "fixture")
            .env("CARGO_PKG_VERSION", "1.2.3")
            .env("OUT_DIR", "/tmp/generated")
            .env("CARGO_MANIFEST_DIR", "/tmp/fixture")
            .env("CARGO_REGISTRY_TOKEN", "fixture")
            .env("CARGO_REGISTRIES_FIXTURE_TOKEN", "fixture")
            .env("CARGO_UNRECOGNISED_CREDENTIAL", "fixture")
            .env("OPENAI_API_KEY", "fixture")
            .output()
            .unwrap();
        assert!(output.status.success());
        let names: Vec<_> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| line.split_once('=').unwrap().0.to_owned())
            .collect();
        assert!(
            names.iter().all(|name| [
                "PATH",
                "TERM",
                "CARGO_PKG_NAME",
                "CARGO_PKG_VERSION",
                "OUT_DIR",
                "CARGO_MANIFEST_DIR"
            ]
            .contains(&name.as_str())),
            "{names:?}"
        );
        assert!(
            [
                "CARGO_PKG_NAME",
                "CARGO_PKG_VERSION",
                "OUT_DIR",
                "CARGO_MANIFEST_DIR"
            ]
            .iter()
            .all(|name| names.iter().any(|actual| actual == name))
        );
    }
}
#[test]
fn shell_startup_is_isolated() {
    let root = tempfile::tempdir().unwrap();
    let sentinel = root.path().join("startup-was-sourced");
    let startup = root.path().join("startup");
    fs::write(&startup, format!("touch '{}'\n", sentinel.display())).unwrap();
    let output = bounded("5s")
        .arg(sanitizer())
        .arg("/usr/bin/env")
        .env("BASH_ENV", startup)
        .env("BASH_FUNC_fixture%%", "() { :; }")
        .env("SHELLOPTS", "xtrace")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!sentinel.exists());
    assert!(output.stderr.is_empty());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .all(|line| ["PATH", "TERM"].contains(&line.split_once('=').unwrap().0))
    );
}
#[test]
fn command_failures_preserve_status() {
    assert_eq!(clean_command(&[]).status.code(), Some(2));
    assert_eq!(
        clean_command(&["/quecto-fixture-absent-command"])
            .status
            .code(),
        Some(127)
    );
    assert_eq!(
        clean_command(&["/bin/sh", "-c", "exit 23"]).status.code(),
        Some(23)
    );
}
#[test]
fn deadline_sigkills_descendant_group() {
    let started = std::time::Instant::now();
    let output = bounded("0.05s")
        .args(["/bin/sh", "-c", "sleep 60 & wait"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(started.elapsed() < std::time::Duration::from_secs(3));
}
#[test]
fn real_offline_compilation_preserves_generated_metadata_not_entry_auth() {
    let root = tempfile::tempdir().unwrap();
    let tools = root.path().join("tools");
    fs::create_dir(&tools).unwrap();
    // Resolve actual tool binaries without inheriting authentication into builds.
    for name in ["cargo", "rustc"] {
        let found = Command::new("timeout")
            .args(["--signal=KILL", "5s", "rustup", "which", name])
            .output()
            .unwrap();
        assert!(found.status.success(), "real Rust toolchain required");
        let binary = String::from_utf8(found.stdout).unwrap();
        let wrapper = tools.join(name);
        fs::write(
            &wrapper,
            format!(
                "#!/bin/bash -p\nexec '{}' '{}' \"$@\"\n",
                sanitizer().display(),
                binary.trim()
            ),
        )
        .unwrap();
        fs::set_permissions(wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(root.path().join("Cargo.toml"), "[package]\nname=\"metadata-fixture\"\nversion=\"0.1.2\"\nedition=\"2021\"\nbuild=\"build.rs\"\n").unwrap();
    fs::write(root.path().join("build.rs"), r#"fn main() {
        assert_eq!(env!("CARGO_PKG_NAME"), "metadata-fixture");
        for name in ["CARGO_REGISTRIES_FIXTURE_TOKEN", "CARGO_REGISTRY_TOKEN", "OPENAI_API_KEY", "UNRECOGNISED_PROVIDER_CREDENTIAL"] {
            assert!(std::env::var_os(name).is_none(), "credential name leaked");
        }
        let out = std::env::var("OUT_DIR").unwrap();
        std::fs::write(std::path::Path::new(&out).join("generated.rs"), "pub const GENERATED: &str = \"generated\";").unwrap();
    }"#).unwrap();
    fs::write(
        root.path().join("src/main.rs"),
        r#"include!(concat!(env!("OUT_DIR"), "/generated.rs"));
    fn main() {
        assert_eq!(env!("CARGO_PKG_NAME"), "metadata-fixture");
        assert_eq!(env!("CARGO_PKG_VERSION"), "0.1.2");
        assert_eq!(GENERATED, "generated");
    }"#,
    )
    .unwrap();
    let output = bounded("30s")
        .arg(tools.join("cargo"))
        .args(["run", "--offline", "--manifest-path"])
        .arg(root.path().join("Cargo.toml"))
        .env("PATH", format!("{}:/usr/bin:/bin", tools.display()))
        .env("HOME", root.path())
        .env("CARGO_HOME", root.path().join("cargo-home"))
        .env(
            "RUSTUP_HOME",
            std::env::var("RUSTUP_HOME").unwrap_or_else(|_| "/opt/rustup".into()),
        )
        .env("CARGO_PKG_NAME", "forged-entry-name")
        .env("OUT_DIR", "forged-entry-dir")
        .env("CARGO_REGISTRY_TOKEN", "fixture")
        .env("CARGO_REGISTRIES_FIXTURE_TOKEN", "fixture")
        .env("OPENAI_API_KEY", "fixture")
        .env("UNRECOGNISED_PROVIDER_CREDENTIAL", "fixture")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
