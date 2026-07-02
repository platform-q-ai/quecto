mod common;

use common::{assert_reviewer_finder_waves, read_repo_file};
use serde_json::Value;

fn read_workflow_config() -> Value {
    serde_json::from_str(&read_repo_file("examples/config.json"))
        .expect("examples/config.json should parse as JSON")
}

fn feature_template(config: &Value) -> &Value {
    config["workflow"]["templates"]
        .as_array()
        .expect("workflow templates should be an array")
        .iter()
        .find(|template| template["id"] == "feature")
        .expect("feature workflow template should exist")
}

fn workflow_steps(config: &Value) -> &[Value] {
    feature_template(config)["steps"]
        .as_array()
        .expect("feature workflow steps should be an array")
}

fn workflow_guards(config: &Value) -> &[Value] {
    feature_template(config)["guards"]
        .as_array()
        .expect("feature workflow guards should be an array")
}

fn assert_reference_steps(steps: &[Value]) {
    // #950 added a `version_bump` step (after `verify`, before `commit`), taking
    // the reference workflow to 19 steps.
    assert_eq!(steps.len(), 19);
    assert_eq!(steps.first().unwrap()["key"], "hooks");
    assert_eq!(
        steps.first().unwrap()["label"],
        "Install/check local quality hooks"
    );
    assert_eq!(steps[1]["key"], "scenarios");
    assert_eq!(steps[3]["key"], "red");
    assert_eq!(steps[4]["key"], "bdd_review");
    // #950: version_bump sits after verify (7) and before commit (9).
    assert_eq!(steps[7]["key"], "verify");
    assert_eq!(steps[8]["key"], "version_bump");
    assert_eq!(steps[9]["key"], "commit");
    // #886: the `merge` and `pull` hand-off steps are removed; the workflow now
    // ends at `pre_merge` (report the PR, do NOT merge) then `cleanup`.
    assert!(steps.iter().all(|s| s["key"] != "merge"));
    assert!(steps.iter().all(|s| s["key"] != "pull"));
    assert_eq!(steps[17]["key"], "pre_merge");
    assert_eq!(
        steps[17]["label"],
        "Confirm the pre-push gate passed and report the PR (do NOT merge)"
    );
    assert_eq!(steps.last().unwrap()["key"], "cleanup");
    assert_eq!(steps.last().unwrap()["label"], "Clean up sub agents");
}

fn assert_reference_guards(guards: &[Value]) {
    assert_eq!(guards.len(), 2);
    assert_eq!(guards[0]["commands"][0], "git commit");
    assert_eq!(guards[0]["commands"][1], "git push");
    assert_eq!(guards[0]["before_step_key"], "commit");
    assert_eq!(guards[1]["commands"][0], "git merge");
    assert_eq!(guards[1]["commands"][1], "gh pr merge");
    // #886: the `merge` step is gone; the no-merge guard re-points at `cleanup`.
    assert_eq!(guards[1]["before_step_key"], "cleanup");
}

#[test]
fn readme_workflow_config_uses_guards_not_deprecated_fields() {
    let readme = read_repo_file("README.md");

    assert!(readme.contains("\"templates\": []"));
    assert!(!readme.contains("\"guard_commit\": true"));
    assert!(!readme.contains("\"enforce_commit_after_step\": 6"));
}

#[test]
fn readme_lists_full_19_step_reference_workflow() {
    let readme = read_repo_file("README.md");

    for expected in [
        "1 - Install/check local quality hooks",
        "2 - Update Scenarios / Add new features",
        "3 - Write/update unit tests (run a quick smoke check; full suite runs on push)",
        "4 - Ensure new/modified tests FAIL (RED) — quick targeted run only, not full suite",
        "5 - Despatch BDD sub-agent to review BDD feature, step tests and unit tests",
        "6 - Implement code (GREEN)",
        "7 - Refactor (perf, security, clean arch)",
        "8 - Ensure tests still pass (GREEN)",
        "9 - Bump semver for every changed crate and sync version docs",
        "10 - Commit",
        "11 - Push (pre-push hook will run tests and linting)",
        "12 - Create PR",
        "13 - Despatch sub agents in parallel as reviewers (Architecture, Security and Performance)",
        "14 - Fix all valid review concerns",
        "15 - Push changes to remote",
        "16 - Reply to the reviewers comments on the PR and mark resolved (use graphql)",
        "17 - Verify the PR meets every issue acceptance criterion",
        "18 - Confirm the pre-push gate passed and report the PR (do NOT merge)",
        "19 - Clean up sub agents",
    ] {
        assert!(
            readme.contains(expected),
            "README missing workflow step: {expected}"
        );
    }
}

#[test]
fn workflow_guide_reference_example_matches_reference_tail() {
    let guide = read_repo_file("docs/workflow.md");

    assert!(guide.contains("\"key\": \"commit\""));
    assert!(guide.contains("\"label\": \"Commit\""));
    assert!(guide.contains("\"phase\": \"ci_cd\""));
    assert!(guide.contains("\"before_step_key\": \"commit\""));
    assert!(guide.contains("\"before_step_key\": \"deploy_prod\""));
}

#[test]
fn workflow_guide_persistence_notes_match_runtime_behavior() {
    let guide = read_repo_file("docs/workflow.md");

    assert!(guide.contains("WorkflowRun` is persisted as first-class session metadata"));
    assert!(guide.contains("template_id`, `done` vector, and `active_issue` survive restarts"));
    assert!(guide.contains("Guards are a developer convenience, not a security boundary"));
}

#[test]
fn examples_config_contains_full_reference_workflow() {
    let config = read_workflow_config();

    assert_reference_steps(workflow_steps(&config));
    assert_reference_guards(workflow_guards(&config));
}

fn reviewers_guidance(config: &Value) -> String {
    workflow_steps(config)
        .iter()
        .find(|s| s["key"] == "reviewers")
        .expect("feature template should have a `reviewers` step")["guidance"]
        .as_str()
        .expect("reviewers step should have string guidance")
        .to_string()
}

// Issue #1004: the mirror tests reuse `common::assert_reviewer_finder_waves`
// — the same helper the native-config test runs — so the three copies are
// pinned to the identical token set and a mirror cannot silently drop an
// angle, wave or verdict semantic the native config carries.

#[test]
fn examples_config_reviewers_describe_finder_waves() {
    // Issue #1004: the mirror config must carry the same three-wave structure.
    let config = read_workflow_config();
    let g = reviewers_guidance(&config);
    assert_reviewer_finder_waves(&g, "examples/config.json");
    // #862 review (Low): the single parallel batch (wall-clock) invariant phrasing
    // must match the native config so this mirror cannot silently drop it.
    assert!(
        g.contains("SINGLE parallel batch"),
        "examples/config.json reviewers guidance should keep the SINGLE parallel batch invariant"
    );
    assert_reviewer_pr_number_hardening(&g, "examples/config.json");
}

fn assert_reviewer_pr_number_hardening(g: &str, source: &str) {
    let lower = g.to_lowercase();
    assert!(
        lower.contains("must not be dispatched before a pr exists")
            || lower.contains("must not dispatch reviewers before a pr exists"),
        "{source} reviewers guidance should block dispatch before a PR exists: {g}"
    );
    assert!(
        lower.contains("forbid") && lower.contains("raw diff"),
        "{source} reviewers guidance should explicitly forbid passing a raw diff: {g}"
    );
    assert!(
        g.contains("PR number") && g.contains("gh pr diff <PR>"),
        "{source} reviewers guidance should require PR number + gh pr diff <PR>: {g}"
    );
    assert!(
        lower.contains("inline") && lower.contains("on the pr"),
        "{source} reviewers guidance should require inline comments on the PR: {g}"
    );
}

/// Extract the `reviewers` step's `guidance` value out of the embedded config in
/// `docs/workflow.md`, so dimension assertions are scoped to the reviewers
/// guidance block rather than matching a word anywhere in the document (which
/// would be a false-pass guard — the file embeds the whole config plus prose).
fn doc_reviewers_guidance(guide: &str) -> &str {
    let after_key = guide
        .split_once("\"key\": \"reviewers\"")
        .expect("docs/workflow.md should embed the reviewers step")
        .1;
    let after_guidance = after_key
        .split_once("\"guidance\": \"")
        .expect("reviewers step in docs should have a guidance field")
        .1;
    // The guidance is a JSON string; scan to its closing quote, skipping escaped
    // `\"` — the read-only instruction embeds `["write", "edit"]`, so a naive
    // split on the first `"` would truncate before the dimensions. The embedded
    // config is ASCII (non-ASCII is `\u`-escaped), so byte indexing is safe.
    let bytes = after_guidance.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return &after_guidance[..i],
            _ => i += 1,
        }
    }
    panic!("reviewers guidance should be a closed JSON string");
}

fn step_guidance<'a>(config: &'a Value, key: &str) -> &'a str {
    workflow_steps(config)
        .iter()
        .find(|s| s["key"] == key)
        .unwrap_or_else(|| panic!("feature template should have a `{key}` step"))["guidance"]
        .as_str()
        .unwrap_or_else(|| panic!("step `{key}` should have string guidance"))
}

#[test]
fn examples_config_mirrors_bdd_strictness_and_version_bump() {
    // #953 + #950: the examples/config.json mirror must carry the strengthened BDD
    // section and the version_bump step.
    let config = read_workflow_config();

    let scenarios = step_guidance(&config, "scenarios").to_lowercase();
    assert!(
        scenarios.contains("declarative") && scenarios.contains("one behaviour per scenario"),
        "examples/config.json scenarios should teach best-practice Gherkin"
    );

    let bdd = step_guidance(&config, "bdd_review").to_lowercase();
    assert!(
        bdd.contains("strict")
            && bdd.contains("every valid")
            && bdd.contains("regardless of severity"),
        "examples/config.json bdd_review should be strict and fix every valid concern"
    );

    let fix = step_guidance(&config, "fix_reviews").to_lowercase();
    assert!(
        fix.contains("every valid") && fix.contains("regardless of severity"),
        "examples/config.json fix_reviews should require fixing every valid concern"
    );

    // #950 version_bump present between verify and commit.
    let vb = step_guidance(&config, "version_bump");
    assert!(
        vb.contains("semver")
            && vb.contains("Current version:")
            && vb.contains("repo_docs.feature"),
        "examples/config.json version_bump should bump semver and sync version docs"
    );
}

#[test]
fn examples_config_mirrors_bdd_finders_and_per_assertion_red() {
    // Issue #1004: the mirror config carries the three narrow bdd_review finders
    // and the per-assertion RED evidence gate.
    let config = read_workflow_config();

    let bdd = step_guidance(&config, "bdd_review");
    for token in ["Gherkin discipline", "Falsifiability", "Coverage"] {
        assert!(
            bdd.contains(token),
            "examples/config.json bdd_review should name the `{token}` finder"
        );
    }
    let bdd_lower = bdd.to_lowercase();
    assert!(
        bdd_lower.contains("quote the offending line"),
        "examples/config.json bdd_review findings must quote the offending line"
    );
    assert!(
        bdd_lower.contains("both sides"),
        "examples/config.json bdd_review coverage finder must pin both sides of limits"
    );

    let red = step_guidance(&config, "red").to_lowercase();
    assert!(
        (red.contains("per new then step") || red.contains("every new then step"))
            && (red.contains("per new test assertion") || red.contains("every new assertion"))
            && red.contains("individually be shown to fail"),
        "examples/config.json red step must require per-assertion failure evidence"
    );
}

#[test]
fn workflow_guide_mirrors_finder_waves_and_per_assertion_red() {
    // docs/workflow.md must embed the #1004 wording too.
    let guide = read_repo_file("docs/workflow.md");
    for token in [
        "Gherkin discipline",
        "Removed-behavior audit",
        "Cross-file tracer",
        "concrete failure scenario",
    ] {
        assert!(
            guide.contains(token),
            "docs/workflow.md should embed the #1004 token `{token}`"
        );
    }
    // "individually" alone is a common English word matched against the whole
    // guide; pin the distinctive per-assertion RED gate phrase instead.
    assert!(
        guide.contains("individually be shown to fail"),
        "docs/workflow.md should embed the per-assertion RED evidence gate"
    );
}

#[test]
fn workflow_guide_mirrors_version_bump_and_strict_bdd() {
    // docs/workflow.md embeds the same config; it must show the version_bump step
    // and the strict BDD wording (#953/#950).
    let guide = read_repo_file("docs/workflow.md");
    assert!(
        guide.contains("\"key\": \"version_bump\""),
        "docs/workflow.md should embed the version_bump step"
    );
    assert!(
        guide.contains("one behaviour per scenario"),
        "docs/workflow.md should embed the best-practice Gherkin checklist"
    );
    assert!(
        guide.contains("regardless of severity"),
        "docs/workflow.md should embed the all-valid-concerns rule"
    );
}

#[test]
fn workflow_guide_reviewers_describe_finder_waves() {
    // docs/workflow.md mirrors the native config and must stay in sync (#1004).
    let guide = read_repo_file("docs/workflow.md");
    let g = doc_reviewers_guidance(&guide);
    assert_reviewer_finder_waves(g, "docs/workflow.md");
    // #862 review (Low): keep the single parallel batch invariant phrasing aligned.
    assert!(
        g.contains("SINGLE parallel batch"),
        "docs/workflow.md reviewers guidance should keep the SINGLE parallel batch invariant"
    );
    assert_reviewer_pr_number_hardening(g, "docs/workflow.md");
}
