//! Structural assertions for the native `refactor` workflow template and the
//! shape-based selector in `workflow-config.json`. These encode the lessons of
//! PR #1031: GREEN-first characterization instead of RED, mutation-based
//! falsifiability, a frozen characterization suite, a source-text-assertion
//! ban, a deletion ledger, four-class parity evidence, and shape-based
//! template routing.

mod common;

use common::read_repo_file;
use serde_json::Value;

fn read_native_config() -> Value {
    serde_json::from_str(&read_repo_file("workflow-config.json"))
        .expect("workflow-config.json should parse as JSON")
}

fn template<'a>(config: &'a Value, id: &str) -> &'a Value {
    config["workflow"]["templates"]
        .as_array()
        .expect("workflow templates should be an array")
        .iter()
        .find(|template| template["id"] == id)
        .unwrap_or_else(|| panic!("`{id}` workflow template should exist"))
}

fn steps(config: &Value) -> &[Value] {
    template(config, "refactor")["steps"]
        .as_array()
        .expect("refactor workflow steps should be an array")
}

fn step<'a>(config: &'a Value, key: &str) -> &'a Value {
    steps(config)
        .iter()
        .find(|s| s["key"] == key)
        .unwrap_or_else(|| panic!("refactor template should have a `{key}` step"))
}

fn guidance<'a>(config: &'a Value, key: &str) -> &'a str {
    step(config, key)["guidance"]
        .as_str()
        .unwrap_or_else(|| panic!("step `{key}` should have string guidance"))
}

fn step_index(config: &Value, key: &str) -> usize {
    steps(config)
        .iter()
        .position(|s| s["key"] == key)
        .unwrap_or_else(|| panic!("refactor template should have a `{key}` step"))
}

// --- Selector routing -------------------------------------------------------

#[test]
fn selector_routes_by_issue_shape_not_hardcoded_feature() {
    let config = read_native_config();
    let selector = config["workflow"]["selector_prompt"]
        .as_str()
        .expect("selector_prompt should be a string");
    assert!(
        !selector.contains("single Quecto repository workflow template, 'feature'"),
        "selector must no longer hard-code the feature template"
    );
    for token in ["zero behaviour change", "'refactor'", "'feature'", "split"] {
        assert!(
            selector.contains(token),
            "selector should route by issue shape and mention {token}: {selector}"
        );
    }
}

#[test]
fn templates_have_mutually_exclusive_when_to_use() {
    let config = read_native_config();
    let feature = template(&config, "feature")["when_to_use"]
        .as_str()
        .expect("feature when_to_use");
    let refactor = template(&config, "refactor")["when_to_use"]
        .as_str()
        .expect("refactor when_to_use");
    assert!(
        feature.contains("behaviour-adding") || feature.contains("behaviour-changing"),
        "feature when_to_use should be scoped to behavioural work: {feature}"
    );
    assert!(
        feature.contains("'refactor'"),
        "feature when_to_use should point refactors at the refactor template"
    );
    assert!(
        refactor.contains("zero behaviour change"),
        "refactor when_to_use should require zero behaviour change: {refactor}"
    );
    assert!(
        refactor.contains("split"),
        "refactor when_to_use should require mixed issues to be split"
    );
}

// --- Step ordering ----------------------------------------------------------

#[test]
fn refactor_pipeline_steps_are_ordered() {
    let config = read_native_config();
    let order = [
        "hooks",
        "scope",
        "characterize",
        "mutation",
        "freeze_review",
        "restructure",
        "parity",
        "version_bump",
        "commit",
        "push",
        "pr",
        "reviewers",
        "fix_reviews",
        "push_fixes",
        "resolve_threads",
        "conformance",
        "pre_merge",
        "cleanup",
    ];
    let mut prev = 0;
    for pair in order.windows(2) {
        let (a, b) = (step_index(&config, pair[0]), step_index(&config, pair[1]));
        assert!(a < b, "step `{}` should precede `{}`", pair[0], pair[1]);
        prev = b;
    }
    assert_eq!(
        prev + 1,
        steps(&config).len(),
        "no unexpected trailing steps"
    );
}

// --- Readiness gate + parity contract (scope) -------------------------------

#[test]
fn scope_step_runs_blocking_and_warning_readiness_gate() {
    let config = read_native_config();
    let g = guidance(&config, "scope");
    for token in [
        "READINESS GATE",
        "BLOCKING",
        "WARNING",
        "named consequence",
        "__UNRESOLVED__",
        "draft",
        "approved",
    ] {
        assert!(g.contains(token), "scope guidance should contain `{token}`");
    }
}

#[test]
fn scope_step_keeps_structural_goals_out_of_tests() {
    let config = read_native_config();
    let g = guidance(&config, "scope");
    assert!(
        g.contains("NEVER become test assertions"),
        "structural goals must be review-time checks, not test assertions"
    );
    assert!(
        g.contains("PARITY CONTRACT"),
        "scope must produce a parity contract"
    );
    assert!(
        g.contains("boundary cases") && g.contains("full-set vs visible-window"),
        "parity contract must enumerate boundary cases including full-set vs window"
    );
}

// --- Characterization (GREEN-first) ------------------------------------------

#[test]
fn characterize_step_is_green_first_on_unmodified_code() {
    let config = read_native_config();
    let g = guidance(&config, "characterize");
    assert_eq!(step(&config, "characterize")["phase"], "green");
    assert!(
        g.contains("UNMODIFIED code") && g.contains("pass GREEN"),
        "characterization tests must pass GREEN on the unmodified code"
    );
    assert!(
        g.contains("fails on master means you misunderstood"),
        "a failing characterization test signals a misunderstanding, not RED"
    );
}

#[test]
fn characterize_step_bans_source_text_assertions() {
    let config = read_native_config();
    let g = guidance(&config, "characterize");
    assert!(
        g.contains("HARD BAN") && g.contains("read production source files"),
        "source-text assertions must be banned outright"
    );
    assert!(
        g.contains("verifies the diff, not the behaviour"),
        "the ban should state the rationale"
    );
    assert!(
        g.contains("step body MUST perform that action"),
        "step bodies must perform the actions their titles claim"
    );
    assert!(
        g.contains("render harness"),
        "TUI behaviour must be pinned at the render-harness runtime class"
    );
}

// --- Mutation gate (replaces RED) --------------------------------------------

#[test]
fn mutation_step_replaces_red_with_mutation_evidence() {
    let config = read_native_config();
    let g = guidance(&config, "mutation");
    assert_eq!(step(&config, "mutation")["phase"], "red");
    for token in [
        "per assertion, not per test target",
        "confirm the test FAILS",
        "revert the mutation",
        "clean working tree",
        "mutation log",
        "hollow",
    ] {
        assert!(
            g.contains(token),
            "mutation guidance should contain `{token}`"
        );
    }
}

// --- Freeze review ------------------------------------------------------------

#[test]
fn freeze_review_dispatches_read_only_finders_then_freezes() {
    let config = read_native_config();
    let g = guidance(&config, "freeze_review");
    for token in [
        "read_only: true",
        "workflow_spec",
        "Falsifiability",
        "Coverage",
        "Gherkin discipline",
        "FREEZE",
        "hash",
        "READ-ONLY",
        "gate violations",
    ] {
        assert!(
            g.contains(token),
            "freeze_review guidance should contain `{token}`"
        );
    }
    assert!(
        step_index(&config, "freeze_review") < step_index(&config, "restructure"),
        "the suite must be reviewed and frozen before the refactor begins"
    );
}

// --- Restructure ---------------------------------------------------------------

#[test]
fn restructure_step_extends_abstractions_and_keeps_deletion_ledger() {
    let config = read_native_config();
    let g = guidance(&config, "restructure");
    assert_eq!(step(&config, "restructure")["phase"], "refactor");
    assert!(
        g.contains("EXTEND the abstraction") && g.contains("shoehorning"),
        "restructure must mandate extend-don't-shoehorn"
    );
    assert!(
        g.contains("DELETION LEDGER"),
        "restructure must keep a deletion ledger"
    );
    assert!(
        g.contains("pass UNMODIFIED"),
        "the frozen suite is the oracle and must pass unmodified"
    );
}

// --- Parity evidence ------------------------------------------------------------

#[test]
fn parity_step_requires_all_four_evidence_classes() {
    let config = read_native_config();
    let g = guidance(&config, "parity");
    for token in [
        "Behavioural",
        "Visual",
        "Performance",
        "Quantitative",
        "ZERO modifications",
        "freeze manifest",
        "never the frozen tests",
    ] {
        assert!(
            g.contains(token),
            "parity guidance should contain `{token}`"
        );
    }
}

// --- Reviewers --------------------------------------------------------------------

#[test]
fn reviewers_step_uses_refactor_weighted_angles_and_single_post() {
    let config = read_native_config();
    let g = guidance(&config, "reviewers");
    for token in [
        "DELETION LEDGER",
        "Equivalence tracer",
        "Test falsifiability",
        "source-text assertions",
        "SINGLE parallel batch",
        "SUBMIT the review",
        "submittedAt != null",
    ] {
        assert!(
            g.contains(token),
            "reviewers guidance should contain `{token}`"
        );
    }
}

// --- Conformance --------------------------------------------------------------------

#[test]
fn conformance_verifies_structural_goals_and_greps_for_source_text_tests() {
    let config = read_native_config();
    let g = guidance(&config, "conformance");
    assert!(
        g.contains("verified HERE by code inspection"),
        "structural goals are verified at conformance, not by tests"
    );
    assert!(
        g.contains("NO test asserts on production source text"),
        "conformance must grep for source-text-asserting tests"
    );
    assert!(
        g.contains("CONFORMANCE: PASS") && g.contains("CONFORMANCE: FAIL"),
        "conformance must emit a machine-checkable verdict"
    );
}

// --- Guards -----------------------------------------------------------------------

#[test]
fn guards_block_commit_before_parity_and_forbid_merge() {
    let config = read_native_config();
    let guards = template(&config, "refactor")["guards"]
        .as_array()
        .expect("refactor guards should be an array");
    let commit_guard = guards
        .iter()
        .find(|g| {
            g["commands"]
                .as_array()
                .is_some_and(|c| c.iter().any(|x| x == "git commit"))
        })
        .expect("a git commit guard should exist");
    assert_eq!(
        commit_guard["before_step_key"], "version_bump",
        "commit/push are blocked until parity evidence is complete"
    );
    let merge_guard = guards
        .iter()
        .find(|g| {
            g["commands"]
                .as_array()
                .is_some_and(|c| c.iter().any(|x| x == "gh pr merge"))
        })
        .expect("a merge guard should exist");
    assert_eq!(merge_guard["before_step_key"], "cleanup");
    assert!(
        merge_guard["message"]
            .as_str()
            .expect("merge guard message")
            .contains("does NOT merge"),
        "the workflow never merges"
    );
}

// --- Mirror parity -----------------------------------------------------------------

#[test]
fn example_config_mirrors_native_refactor_template_and_selector() {
    let native = read_native_config();
    let example: Value = serde_json::from_str(&read_repo_file("examples/config.json"))
        .expect("examples/config.json should parse as JSON");
    assert_eq!(
        native["workflow"]["selector_prompt"], example["workflow"]["selector_prompt"],
        "selector_prompt should match between native and example configs"
    );
    assert_eq!(
        template(&native, "refactor"),
        template(&example, "refactor"),
        "refactor template should match between native and example configs"
    );
}
