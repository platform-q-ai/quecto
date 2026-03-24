use serde_json::Value;
use std::fs;
use std::path::PathBuf;

fn repo_file(relative_path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative_path)
}

fn read_repo_file(relative_path: &str) -> String {
    fs::read_to_string(repo_file(relative_path))
        .unwrap_or_else(|e| panic!("failed to read {relative_path}: {e}"))
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
    let config: Value = serde_json::from_str(&read_repo_file("examples/config.json"))
        .expect("examples/config.json should parse as JSON");
    let steps = config["workflow"]["steps"]
        .as_array()
        .expect("workflow steps should be an array");

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
}
