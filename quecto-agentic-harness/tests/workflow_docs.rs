mod common;

use common::{assert_pure_move_refactor_guidance, read_repo_file};
use serde_json::Value;

fn config() -> Value {
    common::canonical_workflow_config()
}

fn feature(config: &Value) -> &Value {
    config["workflow"]["templates"]
        .as_array()
        .expect("workflow templates")
        .iter()
        .find(|t| t["id"] == "feature")
        .expect("feature template")
}

fn steps(config: &Value) -> &[Value] {
    feature(config)["steps"].as_array().expect("feature steps")
}

fn step<'a>(config: &'a Value, key: &str) -> &'a Value {
    steps(config)
        .iter()
        .find(|s| s["key"] == key)
        .unwrap_or_else(|| panic!("missing feature step `{key}`"))
}

fn guidance<'a>(config: &'a Value, key: &str) -> &'a str {
    step(config, key)["guidance"].as_str().expect("guidance")
}

#[test]
fn readme_workflow_config_uses_guards_not_deprecated_fields() {
    let readme = read_repo_file("README.md");
    assert!(readme.contains("\"templates\": []"));
    assert!(!readme.contains("\"guard_commit\": true"));
    assert!(!readme.contains("\"enforce_commit_after_step\": 6"));
}

#[test]
fn readme_documents_pure_move_refactor_pr_boundary() {
    assert_pure_move_refactor_guidance(&read_repo_file("README.md"));
}

#[test]
fn canonical_feature_is_the_planned_feature_pipeline() {
    let c = config();
    let keys: Vec<_> = steps(&c)
        .iter()
        .map(|s| s["key"].as_str().expect("key"))
        .collect();
    assert_eq!(keys.len(), 20);
    for key in [
        "plan_intake",
        "semantic_contract",
        "test_design",
        "test_review",
        "red",
        "green",
        "refactor_harden",
        "local_review",
        "pr_reviewers",
        "conformance",
        "request_ci",
    ] {
        assert!(keys.contains(&key), "missing planned-feature step `{key}`");
    }
    assert!(!keys.contains(&"scenarios"));
    assert!(!keys.contains(&"bdd_review"));
    assert!(!keys.contains(&"reviewers"));
}

#[test]
fn semantic_contract_and_local_review_are_documented_in_guidance() {
    let c = config();
    let semantic = guidance(&c, "semantic_contract");
    assert!(semantic.contains("semantic risk matrix"));
    assert!(semantic.contains("counterexample reviewer"));
    assert!(semantic.contains("fresh verifier"));

    let local = guidance(&c, "local_review");
    assert!(local.contains("complete unpushed branch state"));
    assert!(local.contains("git diff master...HEAD"));
    assert!(local.contains("semantic-contract counterexample review"));
}

#[test]
fn feature_ci_guidance_handles_reset_automation() {
    let c = config();
    let g = guidance(&c, "request_ci");
    assert!(g.contains("merge-requested"));
    assert!(g.contains("Reset Merge Request"));
    assert!(g.contains("Remove stale merge-requested label"));
    assert!(g.contains("headRefOid"));
}

#[test]
fn feature_version_bump_and_no_merge_guards_remain() {
    let c = config();
    let vb = guidance(&c, "version_bump").to_lowercase();
    assert!(vb.contains("patch"));
    assert!(vb.contains("minor"));

    let guards = feature(&c)["guards"].as_array().expect("guards");
    assert!(guards.iter().any(|g| {
        g["commands"].as_array().is_some_and(|commands| {
            commands.iter().any(|v| v == "git merge") && commands.iter().any(|v| v == "gh pr merge")
        })
    }));
}

#[test]
fn workflow_guide_persistence_notes_match_runtime_behavior() {
    let guide = read_repo_file("docs/workflow.md");
    assert!(guide.contains("WorkflowRun` is persisted as first-class session metadata"));
    assert!(guide.contains("template_id`, `done` vector, and `active_issue` survive restarts"));
    assert!(guide.contains("Guards are a developer convenience, not a security boundary"));
}

#[test]
fn workflow_guide_reference_example_keeps_guard_shape() {
    let guide = read_repo_file("docs/workflow.md");
    assert!(guide.contains("\"before_step_key\": \"commit\""));
    assert!(guide.contains("\"before_step_key\": \"deploy_prod\""));
}
