//! Executable local/CI boundary contract. Adapters record commands, never compile.
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

const STATIC_GATES: [&str; 6] = [
    "check-quality.sh",
    "check-golden-manifest.sh",
    "check-layout-ratchet-tests.sh",
    "check-layout-ratchet.sh",
    "check-bdd-quality.sh",
    "check-bdd-tags.sh",
];
const FMT: &str = "cargo fmt --all -- --check";

struct Fixture {
    tmp: tempfile::TempDir,
}

impl Fixture {
    fn new(rust_change: bool) -> Self {
        let tmp = tempfile::Builder::new()
            .prefix("qp.")
            .tempdir_in("/tmp")
            .expect("short isolated scratch");
        for dir in ["repo/scripts", "home/.cargo/bin", "logs", "scratch"] {
            fs::create_dir_all(tmp.path().join(dir)).expect("fixture directory");
        }
        let fixture = Self { tmp };
        // These replace only the fixture's gate implementations, not the hook.
        let gate = r#"#!/usr/bin/env bash
set -euo pipefail
name=${0##*/}
printf '%s\n' "$name" >> "$RECORD/commands"
case "$name" in
 check-quality.sh|check-bdd-quality.sh|check-bdd-tags.sh)
  touch "$RECORD/start-$name"
  if [ "$BARRIER" = yes ]; then
   for other in check-quality.sh check-bdd-quality.sh check-bdd-tags.sh fmt; do
    until [ -f "$RECORD/start-$other" ]; do sleep .01; done
   done
  fi
  if [ "$SLOW" = "$name" ]; then sleep .1; fi
  ;;
esac
touch "$RECORD/done-$name"
case ",$FAIL," in *",$name,"*) exit 23 ;; esac
"#;
        for name in STATIC_GATES {
            fixture.executable(&format!("repo/scripts/{name}"), gate);
        }
        fixture.executable(
            "home/.cargo/bin/cargo",
            r#"#!/usr/bin/env bash
set -euo pipefail
printf 'cargo %s\n' "$*" >> "$RECORD/commands"
case "$*" in
 'fmt --all -- --check')
  touch "$RECORD/start-fmt"
  if [ "$BARRIER" = yes ]; then
   for other in check-quality.sh check-bdd-quality.sh check-bdd-tags.sh fmt; do
    until [ -f "$RECORD/start-$other" ]; do sleep .01; done
   done
  fi
  if [ "$SLOW" = fmt ]; then sleep .1; fi
  touch "$RECORD/done-fmt"
  case ",$FAIL," in *",fmt,"*) exit 23 ;; esac ;;
 'nextest --version') test "$NEXTEST" = yes ;;
 *) exit 0 ;; # Recording-only: no cargo process or test binary is launched.
esac
"#,
        );
        for name in ["rustc", "rustup", "cargo-nextest", "nextest"] {
            fixture.executable(
                &format!("home/.cargo/bin/{name}"),
                r#"#!/usr/bin/env bash
set -euo pipefail
printf '%s %s\n' "${0##*/}" "$*" >> "$RECORD/commands"
# Recording only; direct compiler/runner calls never launch tools here.
exit 0
"#,
            );
        }
        let setup = fixture.command(
            "set -euo pipefail; git init -q -b master; git add scripts; \
             git commit -qm base; git branch fixture-base; \
             mkdir -p quecto-agentic-harness; printf '// changed\n' > quecto-agentic-harness/change.rs; \
             printf 'changed\n' > README.md; git add .; git commit -qm changed",
        );
        assert_success("isolated git setup", &setup);
        // Both source-classification paths use valid local refs, not origin.
        if rust_change {
            let changed = fixture.command("git diff --name-only fixture-base...HEAD");
            assert_success("changed files", &changed);
            assert!(String::from_utf8_lossy(&changed.stdout).contains("change.rs"));
        } else {
            let same = fixture.command("git update-ref refs/heads/fixture-base HEAD");
            assert_success("no Rust change base", &same);
        }
        fixture
    }

    fn executable(&self, name: &str, text: &str) {
        let path = self.tmp.path().join(name);
        fs::write(&path, text).expect("adapter");
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("executable adapter");
    }

    fn process(&self) -> Command {
        let mut command = Command::new("timeout");
        command.args(["-s", "KILL", "15s", "bash"]);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("GIT_") {
                command.env_remove(key);
            }
        }
        let root = self.tmp.path();
        command
            .current_dir(root.join("repo"))
            .env("HOME", root.join("home"))
            .env("XDG_CONFIG_HOME", root.join("home/.config"))
            .env("GIT_CONFIG_GLOBAL", root.join("home/.gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .env("TMPDIR", root.join("scratch"))
            .env("RECORD", root.join("logs"))
            .env("QUECTO_PREPUSH_BASE", "fixture-base")
            .env("FAIL", "")
            .env("BARRIER", "no")
            .env("SLOW", "")
            .env("NEXTEST", "yes");
        command
    }

    fn command(&self, script: &str) -> Output {
        self.process().args(["-c", script]).output().expect("bash")
    }

    fn hook(&self, fail: &str, barrier: bool, slow: &str, nextest: bool) -> Output {
        let hook = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/pre-push.sh");
        assert!(hook.is_file(), "actual production hook exists");
        let output = self
            .process()
            .arg(hook)
            .env("FAIL", fail)
            .env("BARRIER", if barrier { "yes" } else { "no" })
            .env("SLOW", slow)
            .env("NEXTEST", if nextest { "yes" } else { "no" })
            .output()
            .expect("actual bounded hook");
        assert!(
            matches!(output.status.code(), Some(0 | 1)),
            "hook must exit normally, not timeout/fixture failure: {output:?}"
        );
        output
    }

    fn commands(&self) -> Vec<String> {
        fs::read_to_string(self.tmp.path().join("logs/commands"))
            .expect("recorded actual hook commands")
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn assert_finished(&self, names: &[&str]) {
        for name in names {
            assert!(
                self.tmp.path().join(format!("logs/done-{name}")).is_file(),
                "hook returned before independent gate {name} finished"
            );
        }
    }
}

fn assert_success(label: &str, output: &Output) {
    assert!(output.status.success(), "{label}: {output:?}");
}

fn assert_local_allowlist(commands: &[String]) {
    let allowed: Vec<_> = STATIC_GATES.into_iter().chain([FMT]).collect();
    assert!(
        commands
            .iter()
            .all(|command| allowed.contains(&command.as_str())),
        "local hook command must belong to affirmative no-build allowlist {allowed:?}; actual={commands:?}"
    );
    for required in allowed {
        assert_eq!(
            commands.iter().filter(|c| c.as_str() == required).count(),
            1,
            "required fast gate runs exactly once: {required}; {commands:?}"
        );
    }
}

#[test]
fn pre_push_local_commands_are_only_fast_gates_for_rust_changes() {
    let fixture = Fixture::new(true);
    assert_success("actual hook", &fixture.hook("", false, "", true));
    assert_local_allowlist(&fixture.commands());
}

#[test]
fn pre_push_local_commands_are_only_fast_gates_without_rust_changes() {
    let fixture = Fixture::new(false);
    assert_success("actual hook", &fixture.hook("", false, "", false));
    assert_local_allowlist(&fixture.commands());
}

#[test]
fn pre_push_fixture_records_all_required_static_gates() {
    let fixture = Fixture::new(true);
    assert_success("actual hook", &fixture.hook("", false, "", true));
    let commands = fixture.commands();
    for required in STATIC_GATES.into_iter().chain([FMT]) {
        assert_eq!(
            commands.iter().filter(|c| c.as_str() == required).count(),
            1
        );
    }
    println!("actual hook required-gate recording: {commands:?}");
}

#[test]
fn pre_push_each_fast_gate_failure_independently_blocks() {
    for failed in STATIC_GATES.into_iter().chain(["fmt"]) {
        let fixture = Fixture::new(true);
        let output = fixture.hook(failed, false, "", true);
        assert_eq!(output.status.code(), Some(1), "gate {failed}: {output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("quality gates failed"));
        fixture.assert_finished(&[failed]);
        println!("actual hook rejects independent failure: {failed}");
    }
}

#[test]
fn pre_push_concurrent_failures_wait_for_every_independent_gate() {
    let parallel = [
        "check-quality.sh",
        "check-bdd-quality.sh",
        "check-bdd-tags.sh",
        "fmt",
    ];
    // A rendezvous proves overlap without assuming a scheduler/start order.
    // Rotate the last finisher and fail all independent gates simultaneously.
    for slow in parallel {
        let fixture = Fixture::new(true);
        let output = fixture.hook(&parallel.join(","), true, slow, true);
        assert_eq!(output.status.code(), Some(1), "slow={slow}: {output:?}");
        fixture.assert_finished(&parallel);
        println!("actual hook awaited all concurrent failures; delayed gate={slow}");
    }
}

fn job<'a>(workflow: &'a str, name: &str) -> &'a str {
    let marker = format!("\n  {name}:\n");
    let start = workflow.find(&marker).expect("required CI job") + marker.len();
    let tail = &workflow[start..];
    let mut end = 0;
    for line in tail.split_inclusive('\n') {
        let key = line
            .trim_end_matches('\n')
            .strip_prefix("  ")
            .and_then(|line| line.strip_suffix(':'));
        if key.is_some_and(|key| {
            !key.is_empty()
                && key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        }) {
            break;
        }
        end += line.len();
    }
    &tail[..end]
}

fn named_step<'a>(job: &'a str, name: &str) -> &'a str {
    let marker = format!("      - name: {name}\n");
    let start = job.find(&marker).expect("required named CI step") + marker.len();
    let tail = &job[start..];
    let end = tail.find("\n      - ").unwrap_or(tail.len());
    &tail[..end]
}

#[test]
fn pre_push_ci_owns_strict_workspace_standalone_and_compiled_invariants() {
    // Scope refutations: a command moved to a different job or named step
    // must not satisfy the original owner, even with no separating comment.
    let sample = "jobs:\n  owner:\n    steps:\n      - name: intended\n        run: owned\n      - name: other\n        run: displaced\n  unrelated:\n    run: elsewhere\n";
    let owned = job(sample, "owner");
    assert_eq!(
        owned,
        "    steps:\n      - name: intended\n        run: owned\n      - name: other\n        run: displaced\n"
    );
    assert_eq!(named_step(owned, "intended"), "        run: owned");
    let workflow = include_str!("../../../.github/workflows/ci.yml");
    assert!(workflow.contains("types: [labeled]"));
    let static_job = job(workflow, "static-quality");
    let tests_job = job(workflow, "workspace-tests");
    for (name, source) in [
        ("static-quality", static_job),
        ("workspace-tests", tests_job),
    ] {
        assert!(source.contains("if: github.event_name == 'workflow_dispatch' || github.event.label.name == 'merge-requested'"), "merge gating: {name}");
    }
    for command in [
        "cargo clippy --workspace --all-targets --features quecto-agentic-harness/test-support -- -D warnings -W clippy::cognitive_complexity -W clippy::too_many_arguments -W clippy::too_many_lines",
        "cargo clippy -p quecto-tui --lib --bins -- -D warnings",
        "cargo clippy -p quecto-agentic-harness --lib --bins -- -D warnings",
    ] {
        assert!(
            static_job.contains(&format!("- run: {command}\n")),
            "CI static-quality command-step: {command}"
        );
        println!("CI matrix: ci.yml / static-quality / unnamed run step `{command}`");
    }
    let step = "All workspace unit and integration tests except sharded BDD targets";
    let compiled_step = named_step(tests_job, step);
    assert!(
        compiled_step
            .contains("SHAPE=(--workspace --features quecto-agentic-harness/test-support --bins)")
    );
    assert!(compiled_step.contains("cargo nextest run \"${SHAPE[@]}\" --test architecture --test contracts --test integration --test docs --profile ci"));
    println!(
        "CI matrix: ci.yml / workspace-tests / {step}: architecture + contracts + docs (already-green characterization)"
    );
}
