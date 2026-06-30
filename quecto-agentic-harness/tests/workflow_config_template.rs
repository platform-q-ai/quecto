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
}

#[test]
fn merge_guard_requires_conformance() {
    let config = read_native_config();

    // #886: the `merge` step is removed; the no-merge guard now gates `cleanup`.
    let merge_guard = guards(&config)
        .iter()
        .find(|g| g["before_step_key"] == "cleanup")
        .expect("there should be a no-merge guard before cleanup");
    let message = merge_guard["message"]
        .as_str()
        .expect("merge guard should have a message");
    assert!(
        message.to_lowercase().contains("conformance"),
        "merge guard message should reference conformance, got: {message}"
    );
}

#[test]
fn merge_blocks_on_errored_phase_or_bypassed_gate() {
    // Hardening after the #818 incident: the terminal report step must refuse to
    // hand off a clean PR when an upstream review/fix/conformance phase errored,
    // and must reject a push that bypassed the local gate with --no-verify.
    // #886: this guidance now lives on `pre_merge` (the `merge` step was removed).
    let config = read_native_config();
    let merge_guidance = step(&config, "pre_merge")["guidance"]
        .as_str()
        .expect("pre_merge step should have guidance")
        .to_lowercase();
    assert!(
        merge_guidance.contains("errored") || merge_guidance.contains("did not actually run"),
        "merge guidance should block merging when a review/fix/conformance step errored, got: {merge_guidance}"
    );
    assert!(
        merge_guidance.contains("--no-verify") || merge_guidance.contains("bypass"),
        "merge guidance should reject a gate bypassed with --no-verify, got: {merge_guidance}"
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

    for key in ["pr", "pre_merge"] {
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
fn reviewers_step_default_dimensions_include_full_set() {
    // Issue #845: the standard review fan-out must ALWAYS include, at minimum,
    // Architecture, Security, Performance, Correctness, Conformance-to-AC and
    // Test-quality — not gate Correctness/Test-quality on "larger changes".
    let config = read_native_config();
    let g = guidance(&config, "reviewers");

    for dim in [
        "Architecture",
        "Security",
        "Performance",
        "Correctness",
        "Conformance-to-AC",
        "Test-quality",
    ] {
        assert!(
            g.contains(dim),
            "reviewers guidance should list `{dim}` in the default dimension set, got: {g}"
        );
    }

    // The old gating phrasing ("for larger changes") must be gone — these are
    // defaults now, not conditional extras.
    assert!(
        !g.contains("for larger changes"),
        "reviewers guidance should not gate dimensions on 'for larger changes'"
    );
}

#[test]
fn reviewers_step_conformance_reviewer_checks_acceptance_criteria_and_docs() {
    // The Conformance-to-AC reviewer must re-read the issue's acceptance criteria
    // and check each is actually met, including docs/protocol updates.
    let config = read_native_config();
    let g = guidance(&config, "reviewers").to_lowercase();
    assert!(
        g.contains("acceptance criteria"),
        "reviewers guidance should require re-reading the acceptance criteria"
    );
    // Tighter than a bare "docs" substring (which matches "docs/protocol", URLs,
    // etc.): require the Conformance-to-AC clause to explicitly say each AC must be
    // met INCLUDING documentation.
    assert!(
        g.contains("documentation"),
        "Conformance-to-AC reviewer should explicitly check documentation updates"
    );
    assert!(
        g.contains("conformance-to-ac"),
        "reviewers guidance should name the Conformance-to-AC dimension"
    );
}

#[test]
fn reviewers_step_preserves_single_batch_and_submits_non_pending() {
    // #845: expanding the dimension set must NOT change wall-clock — reviewers
    // still go out as ONE parallel batch — and reviews must be SUBMITTED, never
    // left as a PENDING draft (verified via submittedAt, not the author-side view).
    let config = read_native_config();
    let g = guidance(&config, "reviewers");
    assert!(
        g.contains("SINGLE parallel batch") || g.contains("single parallel batch"),
        "reviewers guidance should keep the single parallel batch invariant"
    );
    let lower = g.to_lowercase();
    assert!(
        lower.contains("submit") && lower.contains("pending"),
        "reviewers guidance should require submitting the review (never leaving it PENDING)"
    );
    assert!(
        lower.contains("submittedat"),
        "reviewers guidance should verify submittedAt (not just the author-side view)"
    );
}

#[test]
fn reviewers_step_requires_pr_number_not_raw_diff_and_pr_precondition() {
    // #946: reviewers must review the PR, not an pasted raw diff blob, and must
    // not be dispatched before the PR exists.
    let config = read_native_config();
    let g = guidance(&config, "reviewers");
    let lower = g.to_lowercase();
    assert!(
        lower.contains("must not be dispatched before a pr exists")
            || lower.contains("must not dispatch reviewers before a pr exists"),
        "reviewers guidance should explicitly block dispatch before a PR exists: {g}"
    );
    assert!(
        lower.contains("forbid") && lower.contains("raw diff"),
        "reviewers guidance should explicitly forbid passing a raw diff: {g}"
    );
    assert!(
        g.contains("PR number") && g.contains("gh pr diff <PR>"),
        "reviewers guidance should require passing the PR number and fetching the diff with gh pr diff <PR>: {g}"
    );
    assert!(
        lower.contains("inline") && lower.contains("on the pr"),
        "reviewers guidance should require inline comments on the PR: {g}"
    );
}

#[test]
fn feature_js_dimensions_array_matches_full_default_set() {
    // #862 review (Medium): `.claude/workflows/feature.js`'s `DIMENSIONS` array is
    // the only copy that actually DRIVES execution, yet the config/doc guards do not
    // cover it — so it could silently revert to the trio with every test green,
    // defeating #845. Guard the executable source of truth directly.
    let js = read_repo_file("../.claude/workflows/feature.js");

    // Extract the `const DIMENSIONS = [ ... ]` literal so we assert on the array
    // that runs, not any prose/prompt text elsewhere in the file.
    let after = js
        .split_once("const DIMENSIONS = [")
        .expect("feature.js should declare `const DIMENSIONS = [...]`")
        .1;
    let array = after
        .split_once(']')
        .expect("feature.js DIMENSIONS array should be closed with ]")
        .0;

    for dim in [
        "Architecture",
        "Security",
        "Performance",
        "Correctness",
        "Conformance-to-AC",
        "Test-quality",
    ] {
        assert!(
            array.contains(dim),
            "feature.js DIMENSIONS array should include `{dim}`, got: {array}"
        );
    }

    // The Conformance-to-AC reviewer must re-read the acceptance criteria and check
    // documentation, and reviews must be submitted (never left PENDING) — assert the
    // executable reviewer dispatch block carries this, mirroring the config guards.
    let reviewer_block = js
        .split_once("// Always dispatch the full default dimension set")
        .expect("feature.js should describe and dispatch parallel reviewers")
        .1
        .split_once("// ── Fix reviews")
        .expect("feature.js reviewer block should end before fix phase")
        .0;
    let lower = reviewer_block.to_lowercase();
    assert!(
        lower.contains("acceptance criteria") && lower.contains("documentation"),
        "feature.js Conformance-to-AC prompt should re-read acceptance criteria incl. documentation"
    );
    assert!(
        lower.contains("submittedat") && lower.contains("pending"),
        "feature.js reviewer prompt should require a submitted (non-PENDING) review"
    );
    assert!(
        lower.contains("must not dispatch reviewers before a pr exists")
            || lower.contains("must not be dispatched before a pr exists"),
        "feature.js reviewer prompt should forbid dispatch before a PR exists"
    );
    assert!(
        lower.contains("forbid")
            && lower.contains("raw diff")
            && reviewer_block.contains("PR number")
            && reviewer_block.contains("gh pr diff <PR>"),
        "feature.js reviewer prompt should forbid raw diffs and require PR number + gh pr diff <PR>"
    );
    assert!(
        lower.contains("inline") && lower.contains("on the pr"),
        "feature.js reviewer prompt should require inline comments on the PR"
    );
}

#[test]
fn reviewer_mechanic_deduplicated() {
    let config = read_native_config();

    // The shared spawn -> await -> read mechanic must be documented in exactly
    // one shared location and not re-embedded in both review steps. We assert
    // the structural property ("documented once, not duplicated") rather than
    // pinning it to a specific host field or harness tool name, so the mechanic
    // can be relocated without churning this test.
    //
    // Candidate shared homes: the template `shared_guidance`/`notes`/
    // `description`, or the workflow `selector_prompt`. The two review steps
    // (`bdd_review`, `reviewers`) must reference, not restate, the mechanic.
    let needle = "spawn";

    let mut shared_locations = 0;
    let feature = feature_template(&config);
    for field in ["shared_guidance", "notes", "description"] {
        if feature
            .get(field)
            .and_then(Value::as_str)
            .is_some_and(|s| s.contains("await") && s.contains("get_messages"))
        {
            shared_locations += 1;
        }
    }
    if config["workflow"]["selector_prompt"]
        .as_str()
        .is_some_and(|s| s.contains("await") && s.contains("get_messages"))
    {
        shared_locations += 1;
    }
    assert_eq!(
        shared_locations, 1,
        "the shared sub-agent review mechanic should be documented in exactly one shared location"
    );

    // Neither review step should re-spell the full mechanic; they reference it.
    let bdd = guidance(&config, "bdd_review");
    let reviewers = guidance(&config, "reviewers");
    let bdd_restates = bdd.contains("await") && bdd.contains(needle);
    let reviewers_restates = reviewers.contains("await") && reviewers.contains(needle);
    assert!(
        !bdd_restates && !reviewers_restates,
        "review steps should reference the shared mechanic, not restate the spawn/await flow"
    );
}
