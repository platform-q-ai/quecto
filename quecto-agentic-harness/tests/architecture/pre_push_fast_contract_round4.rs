//! Narrow admission and execution-environment regressions; no commands execute here.
use super::{WORKSPACE_CLIPPY, ci_contract_valid, pre_push_synthetic_ci};

const TUI_CLIPPY: &str = "cargo clippy -p quecto-tui --lib --bins -- -D warnings";
const HARNESS_CLIPPY: &str = "cargo clippy -p quecto-agentic-harness --lib --bins -- -D warnings";

fn baseline() -> String {
    let source = pre_push_synthetic_ci(true);
    assert!(ci_contract_valid(&source), "unmodified synthetic baseline");
    source
}

fn assert_commands_retained(source: &str, changed: &str) {
    // Compare every command line, including the compiled-test block. In
    // particular, retain -D warnings and all workspace complexity flags exactly.
    let commands = |text: &str| {
        text.lines()
            .filter(|line| line.starts_with("        run: ") || line.starts_with("          "))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    // Nested env values are not shell lines: remove only the exact injected
    // value before comparing the unchanged run mappings and block contents.
    let without_env = changed.replace("          RUSTFLAGS: --cap-lints allow\n", "");
    assert_eq!(commands(source), commands(&without_env));
    for command in [WORKSPACE_CLIPPY, TUI_CLIPPY, HARNESS_CLIPPY] {
        let line = format!("        run: {command}");
        assert_eq!(changed.lines().filter(|actual| *actual == line).count(), 1);
    }
}

fn with_branches(branches: &str) -> String {
    let source = baseline();
    let target = "  pull_request:\n    types: [labeled]\n";
    assert_eq!(source.matches(target).count(), 1, "unique trigger target");
    let injected = format!("  pull_request:\n    branches: {branches}\n    types: [labeled]\n");
    let changed = source.replacen(target, &injected, 1);
    assert_eq!(changed.matches(&injected).count(), 1);
    assert_commands_retained(&source, &changed);
    changed
}

fn with_clippy_cap_lints(name: &str, command: &str) -> String {
    let source = baseline();
    let target = format!("      - name: {name}\n        run: {command}\n");
    assert_eq!(source.matches(&target).count(), 1, "unique Clippy target");
    let injected = format!(
        "      - name: {name}\n        env:\n          RUSTFLAGS: --cap-lints allow\n        run: {command}\n"
    );
    let changed = source.replacen(&target, &injected, 1);
    assert_eq!(changed.matches(&injected).count(), 1);
    assert_eq!(
        changed
            .matches("          RUSTFLAGS: --cap-lints allow\n")
            .count(),
        1,
        "only the selected required Clippy step receives the bypass"
    );
    assert_commands_retained(&source, &changed);
    changed
}

#[test]
fn pre_push_ci_round4_actual_workflow_branch_and_env_remain_valid() {
    // Characterize the real workflow's existing environment, not an invented
    // promise that arbitrary new environment variables are harmless.
    let source = include_str!("../../../.github/workflows/ci.yml");
    assert!(source.contains("  pull_request:\n    branches: [master]\n"));
    assert!(source.contains("        env:\n          GH_TOKEN: ${{ github.token }}\n"));
}

#[test]
fn pre_push_ci_round4_accepts_absent_branch_filter() {
    let source = baseline();
    assert_eq!(source.matches("    branches:").count(), 0);
    assert!(source.contains("  pull_request:\n    types: [labeled]\n"));
    assert!(ci_contract_valid(&source));
}

#[test]
fn pre_push_ci_round4_accepts_exact_master_branch_filter() {
    assert!(ci_contract_valid(&with_branches("[master]")));
}

#[test]
fn pre_push_ci_round4_rejects_release_branch_filter() {
    assert!(
        !ci_contract_valid(&with_branches("[release]")),
        "pull_request branches [release] accepted despite excluding master"
    );
}

#[test]
fn pre_push_ci_round4_rejects_never_matching_branch_filter() {
    assert!(
        !ci_contract_valid(&with_branches("[__never_matches__]")),
        "pull_request branches [__never_matches__] accepted despite excluding master"
    );
}

#[test]
fn pre_push_ci_round4_rejects_negated_master_branch_filter() {
    assert!(
        !ci_contract_valid(&with_branches("[master, '!master']")),
        "pull_request branches [master, '!master'] accepted despite excluding master"
    );
}

#[test]
fn pre_push_ci_round4_rejects_workspace_clippy_cap_lints_env() {
    let invalid = with_clippy_cap_lints("Strict workspace Clippy", WORKSPACE_CLIPPY);
    assert!(
        !ci_contract_valid(&invalid),
        "required workspace Clippy RUSTFLAGS --cap-lints allow bypass accepted"
    );
}

#[test]
fn pre_push_ci_round4_rejects_tui_clippy_cap_lints_env() {
    let invalid = with_clippy_cap_lints("Strict standalone quecto-tui Clippy", TUI_CLIPPY);
    assert!(
        !ci_contract_valid(&invalid),
        "required standalone quecto-tui Clippy RUSTFLAGS --cap-lints allow bypass accepted"
    );
}

#[test]
fn pre_push_ci_round4_rejects_harness_clippy_cap_lints_env() {
    let invalid = with_clippy_cap_lints(
        "Strict standalone quecto-agentic-harness Clippy",
        HARNESS_CLIPPY,
    );
    assert!(
        !ci_contract_valid(&invalid),
        "required standalone quecto-agentic-harness Clippy RUSTFLAGS --cap-lints allow bypass accepted"
    );
}
