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

/// What the fake runtime's image holds, and how the runtime misbehaves.
#[derive(Clone, Copy)]
struct Image<'a> {
    /// `None`: the image carries no `ai.quecto.required-tools` label.
    label: Option<&'a str>,
    /// Executables on the image's PATH.
    tools: &'a [&'a str],
    /// Exit code of `image inspect --format` (a runtime that cannot answer).
    inspect_rc: i32,
    /// Exit code every `run --rm` probe is forced to; 0 really runs it.
    run_rc: i32,
    /// `podman` or `docker`: the two look an image up differently.
    cli: &'a str,
    /// The runtime also warns AFTER the container's own output.
    warns_last: bool,
}

impl Default for Image<'_> {
    fn default() -> Self {
        Self {
            label: None,
            tools: &["git"],
            inspect_rc: 0,
            run_rc: 0,
            cli: "podman",
            warns_last: false,
        }
    }
}

struct Preflight {
    status: i32,
    checks: String,
    stderr: String,
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

    fn probes(&self) -> usize {
        self.runtime_log
            .lines()
            .filter(|call| call.starts_with("run "))
            .count()
    }
}

/// A UTF-8 locale whose collation makes `[A-Za-z]` match accented letters,
/// when this host has one.
fn wide_locale() -> Option<&'static str> {
    static LOCALE: std::sync::OnceLock<Option<&'static str>> = std::sync::OnceLock::new();
    *LOCALE.get_or_init(|| {
        let listed = Command::new("locale").arg("-a").output().ok()?;
        let listed = String::from_utf8_lossy(&listed.stdout).to_lowercase();
        ["en_US.UTF-8", "en_GB.UTF-8"].into_iter().find(|locale| {
            let name = locale.to_lowercase().replace("utf-8", "utf8");
            listed
                .lines()
                .any(|line| line.replace("utf-8", "utf8") == name)
        })
    })
}

fn preflight(image: &Image<'_>) -> Preflight {
    invoke(image, true)
}

/// Runs `create.sh` against a fake runtime whose `run` really executes the
/// probe, with PATH narrowed to the image's tools. Every answer of the fake
/// comes with a warning on stderr, as the real runtimes' do. `preflight_only`
/// false is a real create, which dies at the first failed check.
fn invoke(image: &Image<'_>, preflight_only: bool) -> Preflight {
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
        &bin.join(image.cli),
        &format!(
            r#"#!/usr/bin/env bash
printf '%q ' "$@" >> {log:?}; printf '\n' >> {log:?}
echo 'WARN[0000] "/" is not a shared mount, this could cause issues' >&2
if [ "$1" = image ] && [ "$2" = exists ]; then exit 0; fi
if [ "$1" = image ] && [ "$2" = inspect ] && [ "$3" = --format ]; then
  [ {inspect_rc} = 0 ] || {{ echo "Error: inspect refused" >&2; exit {inspect_rc}; }}
  cat {label:?}; printf '\r\n'; exit 0
fi
if [ "$1" = image ] && [ "$2" = inspect ]; then exit 0; fi
if [ "$1" = run ] && [ "$2" = --rm ] && [ "$3" = --pull=never ] && [ "$5" = sh ]; then
  [ {run_rc} = 0 ] || {{ echo "Error: the runtime said no" >&2; exit {run_rc}; }}
  shift 5
  rc=0; PATH={image_bin:?} /bin/sh "$@" || rc=$?
  [ {warns_last} = 0 ] || echo 'time="now" level=warning msg="lingering mount"' >&2
  exit "$rc"
fi
exit 125
"#,
            inspect_rc = image.inspect_rc,
            run_rc = image.run_rc,
            warns_last = u8::from(image.warns_last),
        ),
    );
    executable(&bin.join("gh"), "#!/bin/sh\nexit 1\n");
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let mut command = Command::new(root().join("scripts/container-runtime/docker/create.sh"));
    command.args(["--state-dir", state.to_str().unwrap()]);
    if preflight_only {
        command.arg("--preflight-only");
    } else {
        command.args(["--", "/bin/true", "--socket"]);
        command.arg(base.join("child.sock"));
    }
    // The allowlist must hold in a locale where [A-Za-z] matches more than
    // ASCII; under C or C.UTF-8 it would hold with or without the guard.
    if let Some(locale) = wide_locale() {
        command.env("LC_ALL", locale);
    }
    let output = command
        .env("PATH", path)
        .env("HOME", base)
        .env("QUECTO_CONTAINER_CLI", image.cli)
        .env_remove("QUECTO_DOCKER_IMAGE")
        .env_remove("QUECTO_REPO_CHECK_TIMEOUT")
        .env_remove("QUECTO_ADMISSION_DIR")
        .output()
        .unwrap();
    Preflight {
        status: output.status.code().unwrap(),
        checks: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
        runtime_log: fs::read_to_string(&log).unwrap_or_default(),
    }
}

#[test]
fn an_image_with_no_rust_and_no_label_passes() {
    for cli in ["podman", "docker"] {
        for label in [None, Some("<no value>"), Some("   ")] {
            let run = preflight(&Image {
                label,
                cli,
                ..Image::default()
            });
            assert_eq!(run.status, 0, "{cli} {label:?}: {}", run.checks);
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
    }
}

#[test]
fn an_image_without_git_fails_the_base_check_and_its_tools_are_not_probed() {
    let run = preflight(&Image {
        label: Some("python3"),
        tools: &["python3"],
        ..Image::default()
    });
    assert_eq!(run.status, 1, "{}", run.checks);
    let base = run.line("image-base");
    assert!(base.starts_with("fail\t"), "{base}");
    assert!(base.contains("cannot host an agent: missing git"), "{base}");
    let tools = run.line("required-tools");
    assert!(tools.starts_with("fail\t"), "{tools}");
    assert!(tools.contains("fix the image-base check first"), "{tools}");
    assert_eq!(run.probes(), 1, "{}", run.runtime_log);
}

#[test]
fn a_label_naming_a_tool_the_image_lacks_fails_naming_that_tool() {
    let run = preflight(&Image {
        label: Some("python3 ruff"),
        tools: &["git", "python3"],
        ..Image::default()
    });
    assert_eq!(run.status, 1, "{}", run.checks);
    assert!(run.line("image-base").starts_with("ok\t"), "{}", run.checks);
    let tools = run.line("required-tools");
    assert!(tools.starts_with("fail\t"), "{tools}");
    assert!(tools.contains("missing ruff"), "{tools}");
}

#[test]
fn a_runtime_warning_after_the_answer_does_not_hide_the_missing_tool() {
    const EXIT_NO_IMAGE: i32 = 6;
    let image = Image {
        label: Some("python3 ruff"),
        tools: &["git", "python3"],
        warns_last: true,
        ..Image::default()
    };
    let tools = preflight(&image);
    let line = tools.line("required-tools");
    assert!(line.starts_with("fail\t"), "{line}");
    assert!(line.contains("missing ruff"), "{line}");
    assert!(line.contains("rebuild the image"), "{line}");
    assert_eq!(invoke(&image, false).status, EXIT_NO_IMAGE);
}

#[test]
fn a_label_whose_tools_are_all_present_passes_and_lists_them() {
    // Any whitespace separates names — and the fake, like the real runtimes,
    // warns on stderr and ends its answer with CR LF: neither is a tool.
    let run = preflight(&Image {
        label: Some("  python3\truff \n cargo-nextest "),
        tools: &["git", "python3", "ruff", "cargo-nextest"],
        ..Image::default()
    });
    assert_eq!(run.status, 0, "{}", run.checks);
    let tools = run.line("required-tools");
    assert!(tools.starts_with("ok\t"), "{tools}");
    assert!(tools.contains("python3 ruff cargo-nextest"), "{tools}");
}

#[test]
fn the_limits_are_inclusive() {
    let name = "t".repeat(64);
    let names: Vec<String> = (0..64).map(|i| format!("tool{i}")).collect();
    let mut present: Vec<&str> = names.iter().map(String::as_str).collect();
    present.extend(["git", name.as_str()]);
    for label in [names.join(" "), name.clone()] {
        let run = preflight(&Image {
            label: Some(&label),
            tools: &present,
            ..Image::default()
        });
        assert_eq!(run.status, 0, "{}", run.checks);
    }
    let too_long = "t".repeat(65);
    let run = preflight(&Image {
        label: Some(&too_long),
        tools: &["git", too_long.as_str()],
        ..Image::default()
    });
    assert!(run.line("required-tools").contains("not a tool name"));
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
        "caf\u{e9}",
        "\u{ff21}bc",
    ] {
        let run = preflight(&Image {
            label: Some(label),
            ..Image::default()
        });
        assert_eq!(run.status, 1, "{label}: {}", run.checks);
        let tools = run.line("required-tools");
        assert!(tools.starts_with("fail\t"), "{label}: {tools}");
        assert!(tools.contains("not a tool name"), "{label}: {tools}");
        assert_eq!(
            run.probes(),
            1,
            "{label}: only the base probe may run:\n{}",
            run.runtime_log
        );
        let refused = label.split_whitespace().last().unwrap();
        assert!(
            run.runtime_log
                .lines()
                .filter(|call| call.starts_with("run "))
                .all(|call| !call.contains(refused)),
            "{label} reached the runtime:\n{}",
            run.runtime_log
        );
    }
}

#[test]
fn a_refused_name_is_quoted_at_a_bounded_length() {
    let huge = "$".repeat(5000);
    let run = preflight(&Image {
        label: Some(&huge),
        ..Image::default()
    });
    let tools = run.line("required-tools");
    assert!(tools.contains(&format!("'{}'", "$".repeat(64))), "{tools}");
    assert!(tools.len() < 600, "{}", tools.len());
}

#[test]
fn an_oversized_label_is_refused() {
    let label = vec!["tool"; 65].join(" ");
    let run = preflight(&Image {
        label: Some(&label),
        tools: &["git", "tool"],
        ..Image::default()
    });
    assert_eq!(run.status, 1, "{}", run.checks);
    let tools = run.line("required-tools");
    assert!(tools.starts_with("fail\t"), "{tools}");
    assert!(tools.contains("at most 64"), "{tools}");
}

#[test]
fn a_runtime_that_cannot_read_the_label_is_a_failure_not_a_pass() {
    for (inspect_rc, words) in [(125, "inspect refused"), (124, "did not answer within")] {
        let run = preflight(&Image {
            label: Some("python3"),
            tools: &["git", "python3"],
            inspect_rc,
            ..Image::default()
        });
        assert_eq!(run.status, 1, "{}", run.checks);
        let tools = run.line("required-tools");
        assert!(tools.starts_with("fail\t"), "{tools}");
        assert!(tools.contains(words), "{tools}");
    }
}

#[test]
fn a_probe_the_runtime_could_not_run_is_the_runtimes_failure_not_the_images() {
    for (run_rc, words) in [
        (124, "did not answer within"),
        (
            125,
            "could not run image quecto-dev:local (exit 125): Error: the runtime said no",
        ),
        (127, "has no usable shell (exit 127)"),
    ] {
        let run = preflight(&Image {
            run_rc,
            ..Image::default()
        });
        assert_eq!(run.status, 1, "{}", run.checks);
        let base = run.line("image-base");
        assert!(base.starts_with("fail\t"), "{base}");
        assert!(base.contains(words), "{run_rc}: {base}");
        assert!(!base.contains("install a POSIX shell and git"), "{base}");
    }
}

#[test]
fn a_real_create_dies_with_the_checks_exit_code_before_allocating_anything() {
    const EXIT_NO_RUNTIME: i32 = 3;
    const EXIT_NO_IMAGE: i32 = 6;
    for (image, code, words) in [
        (
            Image {
                tools: &[],
                ..Image::default()
            },
            EXIT_NO_IMAGE,
            "missing git",
        ),
        (
            Image {
                label: Some("ruff"),
                ..Image::default()
            },
            EXIT_NO_IMAGE,
            "missing ruff",
        ),
        (
            Image {
                label: Some("a/b"),
                ..Image::default()
            },
            EXIT_NO_IMAGE,
            "not a tool name",
        ),
        (
            Image {
                inspect_rc: 125,
                ..Image::default()
            },
            EXIT_NO_RUNTIME,
            "could not read the ai.quecto.required-tools label",
        ),
        (
            Image {
                run_rc: 125,
                ..Image::default()
            },
            EXIT_NO_RUNTIME,
            "could not run image",
        ),
    ] {
        let run = invoke(&image, false);
        assert_eq!(run.status, code, "{}", run.stderr);
        assert!(run.stderr.contains(words), "{}", run.stderr);
        assert!(
            !run.runtime_log.contains("--name"),
            "a container was launched:\n{}",
            run.runtime_log
        );
    }
}

/// This repository's own Containerfile keeps the check it had before the
/// doctor went neutral: deleting the label, or mistyping a name in it, would
/// otherwise leave the doctor green with "declares no required tools".
#[test]
fn quectos_own_containerfile_declares_its_eight_rust_tools() {
    let containerfile =
        fs::read_to_string(root().join(".quecto/containers/standard/Containerfile")).unwrap();
    let declared = declared_tools(&containerfile);
    assert_eq!(
        declared,
        ["cargo rustc rustfmt cargo-clippy cargo-nextest cargo-llvm-cov cargo-deny cargo-machete"]
    );
    let tools: Vec<&str> = declared[0].split(' ').collect();
    let run = preflight(&Image {
        label: Some(declared[0]),
        tools: &[&["git"][..], &tools[..]].concat(),
        ..Image::default()
    });
    assert_eq!(run.status, 0, "{}", run.checks);
}

/// The starter `quecto container init` writes into any project is neutral:
/// it installs exactly the general agent tooling #2073 lists — one `RUN`,
/// whose package set is an allowlist — promises no tool, and keeps the
/// adapter's entrypoint contract.
#[test]
fn the_starter_containerfile_is_tooling_neutral() {
    const GENERAL_TOOLING: &[&str] = &[
        "bash",
        "build-essential",
        "ca-certificates",
        "coreutils",
        "curl",
        "fd-find",
        "findutils",
        "gh",
        "git",
        "grep",
        "jq",
        "less",
        "openssh-client",
        "procps",
        "python3",
        "python3-venv",
        "ripgrep",
        "sed",
    ];
    let starter = fs::read_to_string(
        root().join("quecto-agentic-harness/assets/standard-container/Containerfile"),
    )
    .unwrap();
    // Instructions only, with continuation lines joined.
    let instructions: Vec<String> = starter
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
        .replace("\\\n", " ")
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect();
    let runs: Vec<&String> = instructions
        .iter()
        .filter(|line| line.starts_with("RUN "))
        .collect();
    assert_eq!(runs.len(), 1, "{instructions:#?}");
    let install = runs[0]
        .split(" && ")
        .find(|step| step.starts_with("apt-get install "))
        .expect("the one RUN installs packages");
    let mut packages: Vec<&str> = install
        .split(' ')
        .skip(2)
        .filter(|word| !word.starts_with("--"))
        .collect();
    packages.sort_unstable();
    assert_eq!(packages, GENERAL_TOOLING);
    assert!(
        instructions
            .iter()
            .all(|line| !line.contains("ai.quecto.required-tools")),
        "the starter promises no tool: {instructions:#?}"
    );
    for kept in ["ENTRYPOINT []", "CMD []", "WORKDIR /workspace"] {
        assert!(instructions.iter().any(|line| line == kept), "{kept}");
    }
}

/// The values of every `LABEL ai.quecto.required-tools="…"` instruction.
fn declared_tools(containerfile: &str) -> Vec<&str> {
    containerfile
        .lines()
        .filter_map(|line| line.strip_prefix("LABEL ai.quecto.required-tools=\""))
        .map(|rest| rest.strip_suffix('"').expect("a one-line quoted label"))
        .collect()
}
