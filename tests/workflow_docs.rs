mod common;

use common::read_repo_file;
use serde_json::Value;

fn read_workflow_config() -> Value {
    serde_json::from_str(&read_repo_file("examples/config.json"))
        .expect("examples/config.json should parse as JSON")
}

fn workflow_steps(config: &Value) -> &[Value] {
    config["workflow"]["steps"]
        .as_array()
        .expect("workflow steps should be an array")
}

fn workflow_guards(config: &Value) -> &[Value] {
    config["workflow"]["guards"]
        .as_array()
        .expect("workflow guards should be an array")
}

#[test]
fn readme_workflow_config_uses_guards_not_deprecated_fields() {
    let readme = read_repo_file("README.md");

    assert!(readme.contains("\"guards\": ["));
    assert!(!readme.contains("\"guard_commit\": true"));
    assert!(!readme.contains("\"enforce_commit_after_step\": 6"));
}

#[test]
fn readme_lists_full_16_step_reference_workflow() {
    let readme = read_repo_file("README.md");

    assert!(readme.contains("5 - Refactor (perf, security, clean arch)"));
    assert!(readme.contains("6 - Ensure tests still pass (GREEN)"));
    assert!(readme.contains("15 - Merge"));
    assert!(readme.contains("16 - Move to local master and pull"));
}

#[test]
fn workflow_guide_reference_example_matches_reference_tail() {
    let guide = read_repo_file("docs/workflow.md");

    assert!(guide.contains("\"id\": 15, \"label\": \"Merge\", \"phase\": \"ci_cd\""));
    assert!(guide.contains(
        "\"id\": 16, \"label\": \"Move to local master and pull\", \"phase\": \"ci_cd\""
    ));
    assert!(guide.contains("\"before_step\": 7"));
    assert!(guide.contains("\"before_step\": 15"));
}

#[test]
fn workflow_guide_persistence_notes_match_runtime_behavior() {
    let guide = read_repo_file("docs/workflow.md");

    assert!(guide.contains("stored in-memory for the lifetime of the agent process"));
    assert!(!guide.contains("the workflow state is included in the session file"));
}

#[test]
fn examples_config_contains_full_reference_workflow() {
    let config = read_workflow_config();
    let steps = workflow_steps(&config);
    let guards = workflow_guards(&config);

    assert_eq!(steps.len(), 16);
    assert_eq!(steps.first().unwrap()["id"], 1);
    assert_eq!(
        steps.first().unwrap()["label"],
        "Update Scenarios / Add new features"
    );
    assert_eq!(steps.last().unwrap()["id"], 16);
    assert_eq!(
        steps.last().unwrap()["label"],
        "Move to local master and pull"
    );

    assert_eq!(guards.len(), 2);
    assert_eq!(guards[0]["commands"][0], "git commit");
    assert_eq!(guards[0]["commands"][1], "git push");
    assert_eq!(guards[0]["before_step"], 7);
    assert_eq!(guards[1]["commands"][0], "git merge");
    assert_eq!(guards[1]["commands"][1], "gh pr merge");
    assert_eq!(guards[1]["before_step"], 15);
}
