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
    assert_eq!(steps.len(), 18);
    assert_eq!(steps.first().unwrap()["key"], "hooks");
    assert_eq!(
        steps.first().unwrap()["label"],
        "Install/check local quality hooks"
    );
    assert_eq!(steps[1]["key"], "scenarios");
    assert_eq!(steps[16]["key"], "pull");
    assert_eq!(steps[16]["label"], "Move to local master and pull");
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
    assert_eq!(guards[1]["before_step_key"], "merge");
}

#[test]
fn readme_workflow_config_uses_guards_not_deprecated_fields() {
    let readme = read_repo_file("README.md");

    assert!(readme.contains("\"templates\": []"));
    assert!(!readme.contains("\"guard_commit\": true"));
    assert!(!readme.contains("\"enforce_commit_after_step\": 6"));
}

#[test]
fn readme_lists_full_18_step_reference_workflow() {
    let readme = read_repo_file("README.md");

    for expected in [
        "1 - Install/check local quality hooks",
        "2 - Update Scenarios / Add new features",
        "3 - Write/update unit tests (run a quick smoke check; full suite runs on push)",
        "4 - Ensure new/modified tests FAIL (RED) — quick targeted run only, not full suite",
        "5 - Implement code (GREEN)",
        "6 - Refactor (perf, security, clean arch)",
        "7 - Ensure tests still pass (GREEN)",
        "8 - Commit",
        "9 - Push (pre-push hook will run tests and linting)",
        "10 - Create PR",
        "11 - Despatch sub agents in parallel as reviewers (Architecture, Security and Performance)",
        "12 - Fix all valid review concerns",
        "13 - Push changes to remote",
        "14 - Reply to the reviewers comments on the PR and mark resolved (use graphql)",
        "15 - Confirm the pre-push gate passed (real-LLM, machete, deny run on push)",
        "16 - Merge",
        "17 - Move to local master and pull",
        "18 - Clean up sub agents",
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
