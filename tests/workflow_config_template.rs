//! Structural assertions for the native `feature` workflow template in
//! `workflow-config.json`. These encode issue #814 acceptance criteria:
//! a `conformance` gate, corrected gate/required-check facts, git-add safety,
//! per-step exit criteria, de-duplicated reviewer mechanics, and repo gotchas.

mod common;

use common::read_repo_file;
use serde_json::Value;

fn read_native_config() -> Value {
    serde_json::from_str(&read_repo_file("workflow-config.json"))
        .expect("workflow-config.json should parse as JSON")
}

fn feature_template(config: &Value) -> &Value {
    config["workflow"]["templates"]
        .as_array()
        .expect("workflow templates should be an array")
        .iter()
        .find(|template| template["id"] == "feature")
        .expect("feature workflow template should exist")
}

fn steps(config: &Value) -> &[Value] {
    feature_template(config)["steps"]
        .as_array()
        .expect("feature workflow steps should be an array")
}

fn guards(config: &Value) -> &[Value] {
    feature_template(config)["guards"]
        .as_array()
        .expect("feature workflow guards should be an array")
}

fn step<'a>(config: &'a Value, key: &str) -> &'a Value {
    steps(config)
        .iter()
        .find(|s| s["key"] == key)
        .unwrap_or_else(|| panic!("feature template should have a `{key}` step"))
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
        .unwrap_or_else(|| panic!("feature template should have a `{key}` step"))
}

#[test]
fn config_is_valid_json() {
    // read_native_config panics if not valid JSON.
    let _ = read_native_config();
}

#[test]
fn conformance_step_present_before_merge() {
    let config = read_native_config();

    let conformance = step(&config, "conformance");
    assert_eq!(
        conformance["phase"], "review",
        "conformance step should be in the review phase"
    );

    let g = guidance(&config, "conformance");
    assert!(
        g.contains("CONFORMANCE: PASS"),
        "conformance guidance should describe the CONFORMANCE: PASS verdict"
    );
    assert!(
        g.contains("CONFORMANCE: FAIL"),
        "conformance guidance should describe the CONFORMANCE: FAIL verdict"
    );
    assert!(
        g.contains("file:line"),
        "conformance guidance should require file:line evidence"
    );

    // Positioned after resolve_threads, before pre_merge and merge.
    assert!(
        step_index(&config, "resolve_threads") < step_index(&config, "conformance"),
        "conformance should come after resolve_threads"
    );
    assert!(
        step_index(&config, "conformance") < step_index(&config, "pre_merge"),
        "conformance should come before pre_merge"
    );
    assert!(
        step_index(&config, "conformance") < step_index(&config, "merge"),
        "conformance should come before merge"
    );
}

#[test]
fn merge_guard_requires_conformance() {
    let config = read_native_config();

    let merge_guard = guards(&config)
        .iter()
        .find(|g| g["before_step_key"] == "merge")
        .expect("there should be a merge guard");
    let message = merge_guard["message"]
        .as_str()
        .expect("merge guard should have a message");
    assert!(
        message.to_lowercase().contains("conformance"),
        "merge guard message should reference conformance, got: {message}"
    );
}

#[test]
fn no_stale_strings_remain() {
    // The native config and its example mirror must both be free of the stale
    // required-check / opt-out facts, otherwise the acceptance criterion can
    // report GREEN while the strings still live in the mirror config.
    for path in ["workflow-config.json", "examples/config.json"] {
        let raw = read_repo_file(path);
        assert!(
            !raw.contains("Smoke Test"),
            "{path}: the Smoke Test required-check reference should be gone"
        );
        assert!(
            !raw.contains("QUECTO_SKIP_REAL_LLM"),
            "{path}: the obsolete QUECTO_SKIP_REAL_LLM opt-out should be gone"
        );
    }
}

#[test]
fn gate_facts_describe_mock_llm_default_and_opt_in() {
    let config = read_native_config();

    for key in ["push", "pre_merge"] {
        let g = guidance(&config, key);
        assert!(
            g.contains("@mock-llm"),
            "`{key}` guidance should describe the @mock-llm lane"
        );
        assert!(
            g.contains("QUECTO_RUN_REAL_LLM"),
            "`{key}` guidance should describe the QUECTO_RUN_REAL_LLM opt-in"
        );
    }
}

#[test]
fn required_checks_reference_unit_and_mock_e2e() {
    let config = read_native_config();

    for key in ["pr", "pre_merge", "merge"] {
        let g = guidance(&config, key);
        assert!(
            g.contains("Unit Tests"),
            "`{key}` guidance should reference the Unit Tests required check"
        );
        assert!(
            g.contains("Mock LLM E2E Tests"),
            "`{key}` guidance should reference the Mock LLM E2E Tests required check"
        );
    }
}

#[test]
fn commit_has_git_add_safety() {
    let config = read_native_config();
    let g = guidance(&config, "commit");
    assert!(
        g.contains("git add -A") && g.contains("git add ."),
        "commit guidance should warn against git add -A / git add ."
    );
}

#[test]
fn action_steps_carry_done_when_criteria() {
    let config = read_native_config();
    for key in [
        "hooks",
        "scenarios",
        "tests",
        "red",
        "green",
        "verify",
        "push",
        "pr",
        "fix_reviews",
        "resolve_threads",
        "pre_merge",
        "merge",
    ] {
        let g = guidance(&config, key);
        assert!(
            g.contains("Done when"),
            "`{key}` guidance should carry a `Done when` exit criterion"
        );
    }
}

#[test]
fn action_steps_document_failure_handling() {
    // Item 4 also asks for "If it fails …" guidance where useful; assert it on
    // the steps where a failure path is genuinely actionable.
    let config = read_native_config();
    for key in ["red", "verify", "push"] {
        let g = guidance(&config, key);
        assert!(
            g.contains("If it fails") || g.contains("isn't exercising"),
            "`{key}` guidance should describe what to do on failure"
        );
    }
}

#[test]
fn flaky_find_gotcha_documented() {
    let config = read_native_config();
    let documented = ["push", "pre_merge"].iter().any(|key| {
        let g = guidance(&config, key).to_lowercase();
        g.contains("find.feature") && (g.contains("re-run") || g.contains("rerun"))
    });
    assert!(
        documented,
        "the load-flaky find.feature scenario should be documented with a re-run instruction"
    );
}

#[test]
fn scenarios_step_ties_to_conformance() {
    let config = read_native_config();
    let g = guidance(&config, "scenarios").to_lowercase();
    assert!(
        g.contains("conformance"),
        "scenarios guidance should note its acceptance criteria are the conformance checklist"
    );
}

#[test]
fn pre_merge_confirms_inline_findings_and_resolved_threads() {
    let config = read_native_config();
    let g = guidance(&config, "pre_merge").to_lowercase();
    assert!(
        g.contains("inline"),
        "pre_merge should confirm reviewers actually posted inline findings"
    );
    assert!(
        g.contains("resolved") || g.contains("resolve"),
        "pre_merge should confirm review threads are resolved"
    );
}

#[test]
fn reviewer_mechanic_deduplicated() {
    let config = read_native_config();
    let bdd = guidance(&config, "bdd_review");
    let reviewers = guidance(&config, "reviewers");

    // The shared spawn -> await -> read mechanic must live in one place
    // (the selector_prompt), not be re-embedded in both review steps. At most
    // one of the two review steps may still spell out the full mechanic.
    let bdd_has = bdd.contains("get_messages_tail");
    let reviewers_has = reviewers.contains("get_messages_tail");
    assert!(
        !(bdd_has && reviewers_has),
        "the spawn/await/get_messages_tail mechanic should not be duplicated across both bdd_review and reviewers"
    );

    // ...and it must actually be documented somewhere shared, so the mechanic
    // isn't simply deleted from both steps.
    let selector = config["workflow"]["selector_prompt"]
        .as_str()
        .expect("workflow selector_prompt should be a string");
    assert!(
        selector.contains("get_messages_tail") && selector.contains("await"),
        "the shared sub-agent review mechanic should be documented once in selector_prompt"
    );
}
