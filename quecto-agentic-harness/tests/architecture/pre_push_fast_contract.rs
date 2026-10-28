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
    fn new() -> Self {
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
             git commit -qm base",
        );
        assert_success("isolated git setup", &setup);
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
            .env("FAIL", "")
            .env("BARRIER", "no")
            .env("SLOW", "");
        command
    }

    fn command(&self, script: &str) -> Output {
        self.process().args(["-c", script]).output().expect("bash")
    }

    fn hook(&self, fail: &str, barrier: bool, slow: &str) -> Output {
        let hook = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/pre-push.sh");
        assert!(hook.is_file(), "actual production hook exists");
        let output = self
            .process()
            .arg(hook)
            .env("FAIL", fail)
            .env("BARRIER", if barrier { "yes" } else { "no" })
            .env("SLOW", slow)
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
fn pre_push_local_commands_are_exactly_seven_fast_gates() {
    let fixture = Fixture::new();
    assert_success("actual hook", &fixture.hook("", false, ""));
    assert_local_allowlist(&fixture.commands());
}

#[test]
fn pre_push_each_fast_gate_failure_independently_blocks() {
    for failed in STATIC_GATES.into_iter().chain(["fmt"]) {
        let fixture = Fixture::new();
        let output = fixture.hook(failed, false, "");
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
        let fixture = Fixture::new();
        let output = fixture.hook(&parallel.join(","), true, slow);
        assert_eq!(output.status.code(), Some(1), "slow={slow}: {output:?}");
        fixture.assert_finished(&parallel);
        println!("actual hook awaited all concurrent failures; delayed gate={slow}");
    }
}

// Deliberately bounded to ci.yml's two-space jobs, four-space scalar job
// properties and six-space step mappings. Plain unquoted names/commands and
// literal `run: |` blocks are supported, with allowlisted executable shell lines.
// No YAML aliases, folded/quoted scalars, inline comments or shell equivalence.
// Unsupported spelling fails closed for the required identities and commands.
fn unique_value<'a>(lines: &'a [&str], prefix: &str) -> Option<&'a str> {
    let values: Vec<_> = lines
        .iter()
        .filter_map(|line| line.strip_prefix(prefix))
        .collect();
    match values.as_slice() {
        [value] => Some(*value),
        _ => None,
    }
}

fn tolerance_is_required(lines: &[&str], prefix: &str) -> bool {
    let values: Vec<_> = lines
        .iter()
        .filter_map(|line| line.strip_prefix(prefix))
        .collect();
    matches!(values.as_slice(), [] | ["false"])
}

fn required_job<'a>(lines: &'a [&str], id: &str) -> Option<&'a [&'a str]> {
    let marker = format!("  {id}:");
    let starts: Vec<_> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| **line == marker)
        .map(|(i, _)| i)
        .collect();
    let [start] = starts.as_slice() else {
        return None;
    };
    let tail = &lines[start + 1..];
    let end = tail
        .iter()
        .position(
            |line| match line.strip_prefix("  ").and_then(|line| line.chars().next()) {
                None | Some(' ' | '#') => false,
                Some(_) => true,
            },
        )
        .unwrap_or(tail.len());
    Some(&tail[..end])
}

fn required_step(steps: &[&str], name: &str) -> Option<Vec<String>> {
    let starts: Vec<_> = steps
        .iter()
        .enumerate()
        .filter(|(_, line)| line.starts_with("      - "))
        .map(|(i, _)| i)
        .collect();
    let mut matching = Vec::new();
    for (n, start) in starts.iter().enumerate() {
        let end = starts.get(n + 1).copied().unwrap_or(steps.len());
        let normalized: Vec<String> = steps[*start..end]
            .iter()
            .map(|line| match line.strip_prefix("      - ") {
                Some(first) => format!("        {first}"),
                None => (*line).to_owned(),
            })
            .collect();
        let names: Vec<_> = normalized
            .iter()
            .filter_map(|line| line.strip_prefix("        name: "))
            .collect();
        let unique = names.len() == 1;
        if names.contains(&name) {
            matching.push((normalized, unique));
        }
    }
    match matching.as_slice() {
        [(step, true)] => Some(step.clone()),
        _ => None,
    }
}

fn required_commands(step: &[String], name: &str, commands: &[&str]) -> bool {
    let refs: Vec<_> = step.iter().map(String::as_str).collect();
    let Some(run) = unique_value(&refs, "        run: ") else {
        return false;
    };
    let executable: Vec<_> = if run == "|" {
        let start = refs
            .iter()
            .position(|line| *line == "        run: |")
            .expect("unique literal run");
        refs[start + 1..]
            .iter()
            .take_while(|line| line.starts_with("          ") || line.trim().is_empty())
            .filter_map(|line| line.strip_prefix("          "))
            .collect()
    } else {
        vec![run]
    };
    let mut allowed = commands.to_vec();
    if name == COMPILED_NAME {
        allowed.extend([
            "cargo nextest run \"${SHAPE[@]}\" --lib --profile ci",
            "cargo nextest run \"${SHAPE[@]}\" --test parent_loss --test selected_termination --profile ci",
            "cargo test \"${SHAPE[@]}\" --no-fail-fast --test api_bdd --test mcp_bdd",
        ]);
    }
    let active: Vec<_> = executable
        .iter()
        .copied()
        .filter(|line| match line.trim().chars().next() {
            Some('#') | None => false,
            Some(_) => true,
        })
        .collect();
    mapping_keys_supported(
        &refs,
        "        ",
        &["name", "run", "uses", "continue-on-error"],
        &["with", "env"],
    ) && tolerance_is_required(&refs, "        continue-on-error: ")
        && matches!(
            refs.iter()
                .filter_map(|line| line.strip_prefix("        if: "))
                .collect::<Vec<_>>()
                .as_slice(),
            []
        )
        && active.iter().all(|line| allowed.contains(line))
        && commands
            .iter()
            .all(|command| active.iter().filter(|line| **line == *command).count() == 1)
        && commands.windows(2).all(|pair| {
            active.iter().position(|line| *line == pair[0])
                < active.iter().position(|line| *line == pair[1])
        })
}

// Only the current workflow's direct mapping keys and spelling are supported.
// In particular, an unrecognized/quoted tolerance key is not treated as absent.
fn mapping_keys_supported(lines: &[&str], indent: &str, scalar: &[&str], nested: &[&str]) -> bool {
    lines.iter().all(|line| {
        let Some(field) = line.strip_prefix(indent) else {
            return true;
        };
        match field.chars().next() {
            None | Some(' ' | '#') => true,
            Some(_) => {
                let Some((key, value)) = field.split_once(':') else {
                    return false;
                };
                (scalar.contains(&key) && value.starts_with(' '))
                    || (nested.contains(&key) && (value.is_empty() || value.starts_with(' ')))
            }
        }
    })
}

fn labeled_pull_request_trigger(lines: &[&str]) -> bool {
    let starts: Vec<_> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| **line == "on:")
        .map(|(i, _)| i)
        .collect();
    let [start] = starts.as_slice() else {
        return false;
    };
    let tail = &lines[start + 1..];
    let end = tail
        .iter()
        .position(|line| match line.chars().next() {
            None | Some(' ' | '#') => false,
            Some(_) => true,
        })
        .unwrap_or(tail.len());
    required_job(&tail[..end], "pull_request").is_some_and(|pull_request| {
        mapping_keys_supported(pull_request, "    ", &["types", "branches"], &[])
            && unique_value(pull_request, "    types: ") == Some("[labeled]")
    })
}

fn ci_contract_valid(workflow: &str) -> bool {
    let lines: Vec<_> = workflow.lines().collect();
    let required = [
        (
            "static-quality",
            "Static Quality",
            vec![
                ("Strict workspace Clippy", vec![WORKSPACE_CLIPPY]),
                (
                    "Strict standalone quecto-tui Clippy",
                    vec!["cargo clippy -p quecto-tui --lib --bins -- -D warnings"],
                ),
                (
                    "Strict standalone quecto-agentic-harness Clippy",
                    vec!["cargo clippy -p quecto-agentic-harness --lib --bins -- -D warnings"],
                ),
            ],
        ),
        (
            "workspace-tests",
            "Workspace Tests",
            vec![(COMPILED_NAME, vec![SHAPE, COMPILED_COMMAND])],
        ),
    ];
    labeled_pull_request_trigger(&lines)
        && required.into_iter().all(|(id, display, required)| {
            let Some(body) = required_job(&lines, id) else {
                return false;
            };
            let Some(steps_start) = body.iter().position(|line| *line == "    steps:") else {
                return false;
            };
            mapping_keys_supported(
                body,
                "    ",
                &[
                    "name",
                    "if",
                    "runs-on",
                    "timeout-minutes",
                    "continue-on-error",
                ],
                &["steps", "needs", "permissions"],
            ) && unique_value(body, "    name: ") == Some(display)
                && unique_value(body, "    if: ") == Some(ADMISSION)
                && tolerance_is_required(body, "    continue-on-error: ")
                && body.iter().filter(|line| **line == "    steps:").count() == 1
                && required.into_iter().all(|(name, commands)| {
                    required_step(&body[steps_start + 1..], name)
                        .is_some_and(|step| required_commands(&step, name, &commands))
                })
        })
}

const WORKSPACE_CLIPPY: &str = "cargo clippy --workspace --all-targets --features quecto-agentic-harness/test-support -- -D warnings -W clippy::cognitive_complexity -W clippy::too_many_arguments -W clippy::too_many_lines";
const COMPILED_NAME: &str = "All workspace unit and integration tests except sharded BDD targets";
const COMPILED_COMMAND: &str = "cargo nextest run \"${SHAPE[@]}\" --test architecture --test contracts --test integration --test docs --profile ci";
const SHAPE: &str = "SHAPE=(--workspace --features quecto-agentic-harness/test-support --bins)";
const ADMISSION: &str =
    "github.event_name == 'workflow_dispatch' || github.event.label.name == 'merge-requested'";

fn pre_push_synthetic_ci(name_first: bool) -> String {
    let mut source = format!(
        "on:\n  pull_request:\n    types: [labeled]\njobs:\n  static-quality:\n    name: Static Quality\n    if: {ADMISSION}\n    steps:\n"
    );
    for (name, command) in [
        ("Strict workspace Clippy", WORKSPACE_CLIPPY),
        (
            "Strict standalone quecto-tui Clippy",
            "cargo clippy -p quecto-tui --lib --bins -- -D warnings",
        ),
        (
            "Strict standalone quecto-agentic-harness Clippy",
            "cargo clippy -p quecto-agentic-harness --lib --bins -- -D warnings",
        ),
    ] {
        if name_first {
            source.push_str(&format!("      - name: {name}\n        run: {command}\n"));
        } else {
            source.push_str(&format!("      - run: {command}\n        name: {name}\n"));
        }
    }
    source.push_str(&format!("  workspace-tests:\n    name: Workspace Tests\n    if: {ADMISSION}\n    steps:\n      - name: {COMPILED_NAME}\n        run: |\n          {SHAPE}\n          {COMPILED_COMMAND}\n"));
    source
}

#[test]
fn pre_push_ci_synthetic_valid_boundaries_and_false() {
    for name_first in [true, false] {
        let source = pre_push_synthetic_ci(name_first);
        assert!(ci_contract_valid(&source));
        assert!(ci_contract_valid(
            &source
                .replace("    steps:", "    continue-on-error: false\n    steps:")
                .replace(
                    "        run:",
                    "        continue-on-error: false\n        run:"
                )
        ));
        assert!(ci_contract_valid(&source.replace(
            "  workspace-tests:",
            "    continue-on-error: false\n  workspace-tests:"
        )));
        assert!(ci_contract_valid(&format!(
            "{source}      - name: adjacent\n        run: echo adjacent\n  adjacent-job:\n    name: Adjacent\n"
        )));
    }
}

#[test]
fn pre_push_ci_synthetic_rejects_identity_scope_commands_and_tolerance() {
    let source = pre_push_synthetic_ci(false);
    let mut cases = Vec::new();
    for (label, from, to) in [
        ("job identity", "  static-quality:", "  displaced:"),
        ("job display", "name: Static Quality", "name: Other"),
        ("tests display", "name: Workspace Tests", "name: Other"),
        (
            "missing identity",
            "        name: Strict workspace Clippy\n",
            "",
        ),
        (
            "wrong identity",
            "name: Strict workspace Clippy",
            "name: Other",
        ),
        (
            "moved command",
            "      - run: cargo clippy --workspace",
            "      - name: Other\n        run: cargo clippy --workspace",
        ),
        (
            "comments only",
            "      - run: cargo clippy --workspace",
            "      # - run: cargo clippy --workspace",
        ),
        ("strict warning", "-D warnings", ""),
        ("complexity", "-W clippy::too_many_lines", ""),
        ("shape", SHAPE, "SHAPE=(--workspace --bins)"),
        ("target", "--test contracts", ""),
        ("profile", "--profile ci", ""),
        ("admission", ADMISSION, "always()"),
        ("label trigger", "types: [labeled]", "types: [opened]"),
    ] {
        cases.push((label.to_owned(), source.replacen(from, to, 1)));
    }
    for identity in [
        "  static-quality:\n",
        "        name: Strict workspace Clippy\n",
    ] {
        cases.push((
            format!("duplicate {identity}"),
            source.replacen(identity, &identity.repeat(2), 1),
        ));
    }
    for value in ["true", "${{ false }}"] {
        for scope in ["    steps:", "        name: Strict workspace Clippy"] {
            let indent = if scope == "    steps:" {
                "    "
            } else {
                "        "
            };
            cases.push((
                format!("{scope} tolerance {value}"),
                source.replacen(
                    scope,
                    &format!("{indent}continue-on-error: {value}\n{scope}"),
                    1,
                ),
            ));
        }
    }
    for omitted in [
        "--workspace",
        "--all-targets",
        "--features quecto-agentic-harness/test-support",
        "-W clippy::cognitive_complexity",
        "-W clippy::too_many_arguments",
        "--lib",
        "--bins",
        "-p quecto-tui",
        "-p quecto-agentic-harness",
        "--test architecture",
        "--test integration",
        "--test docs",
    ] {
        cases.push((
            format!("omitted {omitted}"),
            source.replacen(omitted, "", 1),
        ));
    }
    for name in [
        "Strict workspace Clippy",
        "Strict standalone quecto-tui Clippy",
        "Strict standalone quecto-agentic-harness Clippy",
        COMPILED_NAME,
    ] {
        cases.push((
            format!("wrong step {name}"),
            source.replace(&format!("name: {name}"), "name: unrelated"),
        ));
        let duplicate = format!("      - name: {name}\n        run: echo displaced\n");
        // Append the duplicate to its owner, before the adjacent job boundary.
        let invalid = if name == COMPILED_NAME {
            format!("{source}{duplicate}")
        } else {
            source.replace(
                "  workspace-tests:",
                &format!("{duplicate}  workspace-tests:"),
            )
        };
        cases.push((format!("duplicate step {name}"), invalid));
    }
    for value in ["true", "${{ false }}"] {
        let compiled = format!("      - name: {COMPILED_NAME}\n");
        cases.push((
            format!("compiled tolerance {value}"),
            source.replace(
                &compiled,
                &format!("{compiled}        continue-on-error: {value}\n"),
            ),
        ));
        cases.push((
            format!("workspace job tolerance {value}"),
            source.replace(
                "    name: Workspace Tests",
                &format!("    continue-on-error: {value}\n    name: Workspace Tests"),
            ),
        ));
    }
    cases.push((
        "moved compiled command".into(),
        source.replace(
            &format!("          {COMPILED_COMMAND}"),
            &format!("      - name: displaced\n        run: {COMPILED_COMMAND}"),
        ),
    ));
    cases.push((
        "compiled comments only".into(),
        source.replace(
            &format!("          {COMPILED_COMMAND}"),
            &format!("          # {COMPILED_COMMAND}"),
        ),
    ));
    cases.push((
        "reversed shape".into(),
        source.replace(
            &format!("          {SHAPE}\n          {COMPILED_COMMAND}"),
            &format!("          {COMPILED_COMMAND}\n          {SHAPE}"),
        ),
    ));
    cases.push((
        "step conditional".into(),
        source.replace(
            "        name: Strict workspace Clippy",
            "        if: false\n        name: Strict workspace Clippy",
        ),
    ));
    for value in ["true", "${{ false }}"] {
        cases.push((
            format!("trailing job tolerance {value}"),
            source.replace(
                "  workspace-tests:",
                &format!("    continue-on-error: {value}\n  workspace-tests:"),
            ),
        ));
    }
    cases.push((
        "job boundary underscore".into(),
        source.replace(
            &format!("      - name: {COMPILED_NAME}"),
            &format!("  _unrelated:\n    steps:\n      - name: {COMPILED_NAME}"),
        ),
    ));
    for (label, invalid) in cases {
        assert!(
            !ci_contract_valid(&invalid),
            "invalid fixture accepted: {label}"
        );
        println!("rejected synthetic fixture: {label}");
    }
}

#[test]
fn pre_push_ci_owns_strict_workspace_standalone_and_compiled_invariants() {
    assert!(ci_contract_valid(include_str!(
        "../../../.github/workflows/ci.yml"
    )));
}

#[test]
fn pre_push_ci_review_rejects_unsupported_tolerance_keys() {
    let source = pre_push_synthetic_ci(false);
    for key in [
        "\"continue-on-error\": true",
        "'continue-on-error': true",
        "continue-on-error : true",
        "continue-on-error:\n      true",
    ] {
        for (marker, indent) in [
            ("    steps:", "    "),
            ("        name: Strict workspace Clippy", "        "),
        ] {
            let invalid = source.replacen(marker, &format!("{indent}{key}\n{marker}"), 1);
            assert!(
                !ci_contract_valid(&invalid),
                "unsupported tolerance accepted: {indent}{key}"
            );
            println!("rejected review tolerance fixture: {indent}{key}");
        }
    }
}

#[test]
fn pre_push_ci_review_rejects_displaced_label_trigger() {
    let source = pre_push_synthetic_ci(false);
    let invalid = source
        .replace("    types: [labeled]", "    types: [opened]")
        .replace("jobs:", "  unrelated:\n    types: [labeled]\njobs:");
    assert!(
        !ci_contract_valid(&invalid),
        "displaced label trigger accepted"
    );
}
