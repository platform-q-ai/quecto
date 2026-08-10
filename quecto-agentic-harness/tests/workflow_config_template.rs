//! Structural assertions for the canonical planned-feature workflow.

mod common;

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

fn index(config: &Value, key: &str) -> usize {
    steps(config)
        .iter()
        .position(|s| s["key"] == key)
        .unwrap_or_else(|| panic!("missing feature step `{key}`"))
}

#[test]
fn canonical_feature_pipeline_is_ordered() {
    let c = config();
    let expected = [
        "hooks",
        "plan_intake",
        "semantic_contract",
        "test_design",
        "test_review",
        "red",
        "green",
        "refactor_harden",
        "local_review",
        "verify",
        "version_bump",
        "commit",
        "push",
        "pr",
        "pr_reviewers",
        "fix_pr_review",
        "resolve_threads",
        "conformance",
        "request_ci",
        "cleanup",
    ];
    let actual: Vec<_> = steps(&c)
        .iter()
        .map(|s| s["key"].as_str().expect("step key"))
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn planned_feature_requires_plan_and_semantic_contract_before_tests() {
    let c = config();
    assert!(index(&c, "plan_intake") < index(&c, "semantic_contract"));
    assert!(index(&c, "semantic_contract") < index(&c, "test_design"));
    let g = guidance(&c, "semantic_contract");
    for token in [
        "semantic risk matrix",
        "pairwise",
        "high-risk interaction",
        "counterexample reviewer",
        "fresh verifier",
        "observable outcome",
    ] {
        assert!(
            g.contains(token),
            "semantic contract missing `{token}`: {g}"
        );
    }
}

#[test]
fn tests_are_reviewed_and_proven_before_green() {
    let c = config();
    assert!(index(&c, "test_design") < index(&c, "test_review"));
    assert!(index(&c, "test_review") < index(&c, "red"));
    assert!(index(&c, "red") < index(&c, "green"));
    let red = guidance(&c, "red");
    assert!(red.contains("each assertion"));
    assert!(red.contains("falsifiability"));
    assert!(red.contains("mutation residue"));
}

#[test]
fn refactor_and_local_review_happen_before_commit() {
    let c = config();
    assert!(index(&c, "green") < index(&c, "refactor_harden"));
    assert!(index(&c, "refactor_harden") < index(&c, "local_review"));
    assert!(index(&c, "local_review") < index(&c, "commit"));
    let g = guidance(&c, "local_review");
    for token in [
        "git diff master...HEAD",
        "git diff --staged",
        "read_only true",
        "semantic-contract counterexample review",
        "must not post to GitHub",
        "rerun the semantic counterexample reviewer",
    ] {
        assert!(g.contains(token), "local review missing `{token}`: {g}");
    }
}

#[test]
fn pull_request_review_and_conformance_precede_ci() {
    let c = config();
    for key in [
        "pr_reviewers",
        "fix_pr_review",
        "resolve_threads",
        "conformance",
    ] {
        assert!(index(&c, key) < index(&c, "request_ci"));
    }
    let review = guidance(&c, "pr_reviewers");
    assert!(review.contains("read-only"));
    assert!(review.contains("submitted GitHub review"));
    let conformance = guidance(&c, "conformance");
    assert!(conformance.contains("semantic risk matrix"));
    assert!(conformance.contains("CONFORMANCE: PASS"));
}

#[test]
fn merge_requested_reset_race_is_explicit() {
    let c = config();
    let g = guidance(&c, "request_ci");
    for token in [
        "headRefOid",
        "merge-requested",
        "Reset Merge Request",
        "Remove stale merge-requested label",
        "If CI fails",
        "re-add",
        "never wait indefinitely",
    ] {
        assert!(g.contains(token), "CI guidance missing `{token}`: {g}");
    }
}

#[test]
fn guards_block_early_push_and_all_merges() {
    let c = config();
    let guards = feature(&c)["guards"].as_array().expect("guards");
    assert!(guards.iter().any(|g| {
        g["before_step_key"] == "commit"
            && g["commands"].as_array().is_some_and(|commands| {
                commands.iter().any(|v| v == "git commit")
                    && commands.iter().any(|v| v == "git push")
            })
    }));
    assert!(guards.iter().any(|g| {
        g["commands"].as_array().is_some_and(|commands| {
            commands.iter().any(|v| v == "git merge") && commands.iter().any(|v| v == "gh pr merge")
        })
    }));
}

#[test]
fn version_bump_is_between_verification_and_commit() {
    let c = config();
    assert!(index(&c, "verify") < index(&c, "version_bump"));
    assert!(index(&c, "version_bump") < index(&c, "commit"));
    let g = guidance(&c, "version_bump").to_lowercase();
    assert!(g.contains("patch"));
    assert!(g.contains("minor"));
}

#[test]
fn every_action_step_has_a_done_condition() {
    let c = config();
    for s in steps(&c) {
        let key = s["key"].as_str().expect("key");
        let g = s["guidance"].as_str().unwrap_or("");
        assert!(
            g.contains("Done when") || g.contains("Done only when"),
            "`{key}` lacks a done condition"
        );
    }
}
