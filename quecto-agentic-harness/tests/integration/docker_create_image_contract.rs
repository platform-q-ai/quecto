#![cfg(unix)]

//! The create adapter's image checks are tooling-neutral (#2073): the
//! harness needs a shell and git inside the image and nothing else. Anything
//! more is the image's own promise, declared in its
//! `ai.quecto.required-tools` label and checked by the doctor.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    let mut p = fs::metadata(path).unwrap().permissions();
    p.set_mode(0o755);
    fs::set_permissions(path, p).unwrap();
}

/// What the fake runtime's image holds.
struct Image<'a> {
    /// `None`: the image carries no `ai.quecto.required-tools` label.
    label: Option<&'a str>,
    /// Executables on the image's PATH.
    tools: &'a [&'a str],
    /// Exit code of `image inspect` (a runtime that cannot answer).
    inspect_rc: i32,
}

struct Preflight {
    status: i32,
    checks: String,
    runtime_log: String,
}

impl Preflight {
    /// The tab-separated doctor line of one check.
    fn line(&self, check: &str) -> &str {
        self.checks
            .lines()
            .find(|line| line.split('\t').nth(1) == Some(check))
            .unwrap_or_else(|| panic!("no `{check}` check in:\n{}", self.checks))
    }
}

/// Runs `create.sh --preflight-only` against a fake podman whose `run`
/// really executes the probe, with PATH narrowed to the image's tools.
fn preflight(image: &Image<'_>) -> Preflight {
    let base = tempfile::tempdir().unwrap();
    let base = base.path();
    let (bin, image_bin, state) = (base.join("bin"), base.join("image-bin"), base.join("state"));
    for dir in [&bin, &image_bin, &state] {
        fs::create_dir_all(dir).unwrap();
    }
    for tool in image.tools {
        executable(&image_bin.join(tool), "#!/bin/sh\nexit 0\n");
    }
    let log = base.join("runtime.log");
    let label = base.join("label");
    fs::write(&label, image.label.unwrap_or("")).unwrap();
    executable(
        &bin.join("podman"),
        &format!(
            r#"#!/usr/bin/env bash
printf '%q ' "$@" >> {log:?}; printf '\n' >> {log:?}
if [ "$1" = image ] && [ "$2" = exists ]; then exit 0; fi
if [ "$1" = image ] && [ "$2" = inspect ]; then
  [ {inspect_rc} = 0 ] || {{ echo "Error: inspect refused" >&2; exit {inspect_rc}; }}
  cat {label:?}; printf '\n'; exit 0
fi
if [ "$1" = run ] && [ "$2" = --rm ] && [ "$3" = --pull=never ] && [ "$5" = sh ]; then
  shift 5
  PATH={image_bin:?} exec /bin/sh "$@"
fi
exit 125
"#,
            inspect_rc = image.inspect_rc,
        ),
    );
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let output = Command::new(root().join("scripts/container-runtime/docker/create.sh"))
        .args(["--state-dir", state.to_str().unwrap(), "--preflight-only"])
        .env("PATH", path)
        .env("HOME", base)
        .env("QUECTO_CONTAINER_CLI", "podman")
        .output()
        .unwrap();
    Preflight {
        status: output.status.code().unwrap(),
        checks: String::from_utf8(output.stdout).unwrap(),
        runtime_log: fs::read_to_string(&log).unwrap_or_default(),
    }
}

#[test]
fn an_image_with_no_rust_and_no_label_passes() {
    let run = preflight(&Image {
        label: None,
        tools: &["git"],
        inspect_rc: 0,
    });
    assert_eq!(run.status, 0, "{}", run.checks);
    assert!(run.line("image-base").starts_with("ok\t"), "{}", run.checks);
    let tools = run.line("required-tools");
    assert!(tools.starts_with("ok\t"), "{tools}");
    assert!(tools.contains("declares no required tools"), "{tools}");
    assert!(
        !run.checks.contains("cargo") && !run.checks.contains("dev-tools"),
        "no check may name a language toolchain:\n{}",
        run.checks
    );
}

#[test]
fn an_image_without_git_fails_the_base_check() {
    let run = preflight(&Image {
        label: None,
        tools: &[],
        inspect_rc: 0,
    });
    assert_eq!(run.status, 1, "{}", run.checks);
    let base = run.line("image-base");
    assert!(base.starts_with("fail\t"), "{base}");
    assert!(base.contains("missing git"), "{base}");
}

#[test]
fn a_label_naming_a_tool_the_image_lacks_fails_naming_that_tool() {
    let run = preflight(&Image {
        label: Some("python3 ruff"),
        tools: &["git", "python3"],
        inspect_rc: 0,
    });
    assert_eq!(run.status, 1, "{}", run.checks);
    assert!(run.line("image-base").starts_with("ok\t"), "{}", run.checks);
    let tools = run.line("required-tools");
    assert!(tools.starts_with("fail\t"), "{tools}");
    assert!(tools.contains("missing ruff"), "{tools}");
}

#[test]
fn a_label_whose_tools_are_all_present_passes_and_lists_them() {
    let run = preflight(&Image {
        label: Some("  python3\truff  cargo-nextest "),
        tools: &["git", "python3", "ruff", "cargo-nextest"],
        inspect_rc: 0,
    });
    assert_eq!(run.status, 0, "{}", run.checks);
    let tools = run.line("required-tools");
    assert!(tools.starts_with("ok\t"), "{tools}");
    assert!(tools.contains("python3 ruff cargo-nextest"), "{tools}");
}

#[test]
fn a_label_that_is_not_a_list_of_tool_names_is_refused_and_never_run() {
    for label in [
        "python3;touch${IFS}pwned",
        "$(id)",
        "-rf",
        "a/b",
        "ok `id`",
        "*",
    ] {
        let run = preflight(&Image {
            label: Some(label),
            tools: &["git"],
            inspect_rc: 0,
        });
        assert_eq!(run.status, 1, "{label}: {}", run.checks);
        let tools = run.line("required-tools");
        assert!(tools.starts_with("fail\t"), "{label}: {tools}");
        assert!(tools.contains("not a tool name"), "{label}: {tools}");
        let probes = run
            .runtime_log
            .lines()
            .filter(|call| call.starts_with("run "))
            .count();
        assert_eq!(
            probes, 1,
            "{label}: only the base probe may run:\n{}",
            run.runtime_log
        );
    }
}

#[test]
fn an_oversized_label_is_refused() {
    let label = vec!["tool"; 65].join(" ");
    let run = preflight(&Image {
        label: Some(&label),
        tools: &["git", "tool"],
        inspect_rc: 0,
    });
    assert_eq!(run.status, 1, "{}", run.checks);
    let tools = run.line("required-tools");
    assert!(tools.starts_with("fail\t"), "{tools}");
    assert!(tools.contains("at most 64"), "{tools}");
}

#[test]
fn a_runtime_that_cannot_read_the_label_is_a_failure_not_a_pass() {
    let run = preflight(&Image {
        label: Some("python3"),
        tools: &["git", "python3"],
        inspect_rc: 125,
    });
    assert_eq!(run.status, 1, "{}", run.checks);
    let tools = run.line("required-tools");
    assert!(tools.starts_with("fail\t"), "{tools}");
    assert!(tools.contains("inspect refused"), "{tools}");
}
