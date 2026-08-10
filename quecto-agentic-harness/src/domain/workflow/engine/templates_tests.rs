use super::*;

fn feature() -> WorkflowTemplate {
    default_templates()
        .into_iter()
        .find(|t| t.id == "feature")
        .expect("the embedded canonical folder must define a `feature` template")
}

#[test]
fn embedded_canonical_folder_parses_into_both_templates() {
    // The whole point of embedding the canonical `workflows/` folder at
    // compile time: the RUNTIME templates are the guard-tested spec, so
    // they cannot drift, and every shared-step reference must resolve.
    let templates = default_templates();
    let ids: Vec<&str> = templates.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "feature",
            "adversarial-review",
            "bugfix",
            "chore",
            "flake-hunt",
            "investigate",
            "plan",
            "prd",
            "refactor",
            "remove",
        ]
    );
    for t in &templates {
        assert!(!t.steps.is_empty(), "template `{}` must have steps", t.id);
    }
}

#[test]
fn shared_step_references_resolve_to_inline_steps() {
    // AC2: `steps/shared/hooks` is defined once on disk and referenced by
    // both templates; after embedding+resolution each template carries a
    // fully inlined `hooks` step with the shared guidance.
    for t in default_templates()
        .into_iter()
        .filter(|t| t.steps.iter().any(|s| s.key == "hooks"))
    {
        let hooks = t
            .steps
            .iter()
            .find(|s| s.key == "hooks")
            .unwrap_or_else(|| panic!("template `{}` must resolve the shared hooks step", t.id));
        assert!(
            hooks
                .guidance
                .as_deref()
                .unwrap_or("")
                .contains("install-hooks.sh"),
            "template `{}` hooks step must carry the shared guidance",
            t.id
        );
    }
}

#[test]
fn runtime_review_steps_are_read_only_at_runtime() {
    // Regression for the `shared_guidance` bug: the reviewer read-only
    // instruction MUST live in a field the runtime actually deserializes —
    // the per-step `guidance` — not a phantom field serde drops. Assert it on
    // the RUNTIME template returned by `default_templates()`, so a future move
    // back into an un-deserialized field fails here, not silently in prod.
    let f = feature();
    for key in [
        "semantic_contract",
        "test_review",
        "local_review",
        "pr_reviewers",
    ] {
        let g = f
            .steps
            .iter()
            .find(|s| s.key == key)
            .and_then(|s| s.guidance.as_deref())
            .unwrap_or("");
        assert!(
            g.contains("read-only") || g.contains("read_only"),
            "runtime `{key}` guidance must instruct read-only reviewer spawns: {g}"
        );
        assert!(
            g.contains("openai-oauth/gpt-5.5") || key == "pr_reviewers",
            "runtime `{key}` guidance must pin the reviewer model when spawning directly: {g}"
        );
    }
}

#[test]
fn runtime_feature_template_has_version_bump_step() {
    // #950 lives in the spec; prove it reaches the runtime template too.
    assert!(feature().steps.iter().any(|s| s.key == "version_bump"));
}
