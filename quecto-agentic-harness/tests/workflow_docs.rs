mod common;

use common::read_repo_file;
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
    assert_eq!(steps.len(), 17);
    assert_eq!(steps.first().unwrap()["key"], "hooks");
    assert_eq!(
        steps.first().unwrap()["label"],
        "Install/check local quality hooks"
    );
    assert_eq!(steps[1]["key"], "scenarios");
    assert_eq!(steps[3]["key"], "red");
    assert_eq!(steps[4]["key"], "bdd_review");
    // #886: the `merge` and `pull` hand-off steps are removed; the workflow now
    // ends at `pre_merge` (report the PR, do NOT merge) then `cleanup`.
    assert!(steps.iter().all(|s| s["key"] != "merge"));
    assert!(steps.iter().all(|s| s["key"] != "pull"));
    assert_eq!(steps[15]["key"], "pre_merge");
    assert_eq!(
        steps[15]["label"],
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
fn readme_lists_full_17_step_reference_workflow() {
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
        "9 - Commit",
        "10 - Push (pre-push hook will run tests and linting)",
        "11 - Create PR",
        "12 - Despatch sub agents in parallel as reviewers (Architecture, Security and Performance)",
        "13 - Fix all valid review concerns",
        "14 - Push changes to remote",
        "15 - Reply to the reviewers comments on the PR and mark resolved (use graphql)",
        "16 - Confirm the pre-push gate passed and report the PR (do NOT merge)",
        "17 - Clean up sub agents",
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

#[test]
fn examples_config_reviewers_default_dimensions_include_full_set() {
    // Issue #845: the mirror config must list the same full default dimension set.
    let config = read_workflow_config();
    let g = reviewers_guidance(&config);
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
            "examples/config.json reviewers guidance should list `{dim}`"
        );
    }
    assert!(
        !g.contains("for larger changes"),
        "examples/config.json reviewers guidance should not gate dimensions on 'for larger changes'"
    );
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
    after_guidance
        .split_once('"')
        .expect("reviewers guidance should be a closed JSON string")
        .0
}

#[test]
fn workflow_guide_reviewers_default_dimensions_include_full_set() {
    // docs/workflow.md mirrors the native config and must stay in sync (#845).
    let guide = read_repo_file("docs/workflow.md");
    let g = doc_reviewers_guidance(&guide);
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
            "docs/workflow.md reviewers guidance should list `{dim}`, got: {g}"
        );
    }
    assert!(
        !g.contains("for larger changes"),
        "docs/workflow.md reviewers guidance should not gate dimensions on 'larger changes'"
    );
    // #862 review (Low): keep the single parallel batch invariant phrasing aligned.
    assert!(
        g.contains("SINGLE parallel batch"),
        "docs/workflow.md reviewers guidance should keep the SINGLE parallel batch invariant"
    );
    assert_reviewer_pr_number_hardening(g, "docs/workflow.md");
}
