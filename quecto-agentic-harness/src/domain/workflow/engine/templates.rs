use crate::domain::workflow::WorkflowTemplate;

/// The built-in workflow templates.
///
/// These are defined ONCE in the canonical `workflows/` folder (the single
/// source of truth per the workflow-composable-templates PRD §3.2 / AC7) and
/// embedded here at compile time, so the RUNNING workflow is byte-for-byte the
/// reviewed spec. Template id = filename stem, exactly as the runtime
/// directory loader (`load_workflow_templates_from_dir`) derives it; the
/// `runtime_default_templates_match_canonical_folder` drift test pins the two
/// load paths to resolve identically.
///
/// Shared steps live once in `workflows/steps/` and are referenced by string
/// path from the template files (slice 1 step-entry union). The embedded
/// resolver below handles exactly the reference forms the canonical files
/// use; anything else in an embedded file is a compile-tested developer error
/// and panics at first use (caught by the unit tests in this module).
///
/// `include_str!` + `serde_json::from_str` is pure compile-time embedding plus
/// in-memory parsing (no filesystem or network access at runtime).
pub fn default_templates() -> Vec<WorkflowTemplate> {
    /// The canonical template files: (id = filename stem, embedded content).
    const TEMPLATE_FILES: &[(&str, &str)] = &[
        (
            "feature",
            include_str!("../../../../workflows/feature.json"),
        ),
        (
            "refactor",
            include_str!("../../../../workflows/refactor.json"),
        ),
    ];
    /// The canonical shared-step files, keyed by their reference path
    /// (relative to the workflow dir, `.json` extension omitted).
    const STEP_FILES: &[(&str, &str)] = &[
        (
            "steps/shared/hooks",
            include_str!("../../../../workflows/steps/shared/hooks.json"),
        ),
        (
            "steps/shared/push_fixes",
            include_str!("../../../../workflows/steps/shared/push_fixes.json"),
        ),
        (
            "steps/shared/resolve_threads",
            include_str!("../../../../workflows/steps/shared/resolve_threads.json"),
        ),
    ];

    TEMPLATE_FILES
        .iter()
        .map(|(id, content)| parse_embedded_template(id, content, STEP_FILES))
        .collect()
}

/// Parse one embedded canonical template file: inject the filename-stem id and
/// resolve string step references against the embedded step library.
fn parse_embedded_template(
    id: &str,
    content: &str,
    step_files: &[(&str, &str)],
) -> WorkflowTemplate {
    let mut value: serde_json::Value = serde_json::from_str(content)
        .unwrap_or_else(|e| panic!("embedded workflow template `{id}` must parse: {e}"));
    let object = value
        .as_object_mut()
        .unwrap_or_else(|| panic!("embedded workflow template `{id}` must be an object"));
    object.insert("id".into(), serde_json::Value::String(id.to_owned()));
    let steps = object
        .get_mut("steps")
        .and_then(serde_json::Value::as_array_mut)
        .unwrap_or_else(|| panic!("embedded workflow template `{id}` must have a steps array"));
    for entry in steps {
        if let serde_json::Value::String(reference) = entry {
            let resolved = step_files
                .iter()
                .find(|(path, _)| {
                    *path == reference.as_str() || format!("{path}.json") == *reference
                })
                .map(|(_, step)| step)
                .unwrap_or_else(|| {
                    panic!("embedded template `{id}` references unembedded step `{reference}`")
                });
            *entry = serde_json::from_str(resolved)
                .unwrap_or_else(|e| panic!("embedded shared step `{reference}` must parse: {e}"));
        }
    }
    serde_json::from_value(value)
        .unwrap_or_else(|e| panic!("embedded workflow template `{id}` must deserialize: {e}"))
}

pub(super) fn phase_display_name(phase: &str) -> &str {
    match phase {
        "red" => "RED",
        "green" => "GREEN",
        "refactor" => "REFACTOR",
        "ci_cd" => "CI/CD",
        "review" => "REVIEW",
        other => other,
    }
}

#[cfg(test)]
mod tests {
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
        assert_eq!(ids, ["feature", "refactor"]);
        for t in &templates {
            assert!(!t.steps.is_empty(), "template `{}` must have steps", t.id);
        }
    }

    #[test]
    fn shared_step_references_resolve_to_inline_steps() {
        // AC2: `steps/shared/hooks` is defined once on disk and referenced by
        // both templates; after embedding+resolution each template carries a
        // fully inlined `hooks` step with the shared guidance.
        for t in default_templates() {
            let template_id = t.id.clone();
            let hooks = t
                .steps
                .iter()
                .find(|s| s.key == "hooks")
                .unwrap_or_else(|| {
                    panic!("template `{template_id}` must resolve the shared hooks step")
                });
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
        for key in ["bdd_review", "reviewers"] {
            let g = f
                .steps
                .iter()
                .find(|s| s.key == key)
                .and_then(|s| s.guidance.as_deref())
                .unwrap_or("");
            assert!(
                g.contains("read_only") && g.contains("[\"write\", \"edit\"]"),
                "runtime `{key}` guidance must instruct read_only spawns disabling [\"write\", \"edit\"]: {g}"
            );
            for keep in ["bash", "read", "grep", "find", "agent_cmd"] {
                assert!(
                    g.contains(keep),
                    "runtime `{key}` guidance must name retained tool `{keep}`: {g}"
                );
            }
        }
    }

    #[test]
    fn runtime_feature_template_has_version_bump_step() {
        // #950 lives in the spec; prove it reaches the runtime template too.
        assert!(feature().steps.iter().any(|s| s.key == "version_bump"));
    }
}

#[cfg(test)]
#[path = "templates_cov_tests.rs"]
mod cov_tests;

#[cfg(test)]
mod coverage_tests {
    use super::*;

    #[test]
    fn phase_display_name_maps_known_phases_and_borrows_unknown() {
        assert_eq!(phase_display_name("red"), "RED");
        assert_eq!(phase_display_name("green"), "GREEN");
        assert_eq!(phase_display_name("refactor"), "REFACTOR");
        assert_eq!(phase_display_name("ci_cd"), "CI/CD");
        assert_eq!(phase_display_name("review"), "REVIEW");
        assert_eq!(phase_display_name("security"), "security");
    }

    #[test]
    fn parse_embedded_template_injects_id_and_accepts_json_extension_references() {
        let template = parse_embedded_template(
            "custom",
            r#"{"label":"Custom","description":"desc","steps":["steps/shared.json"]}"#,
            &[(
                "steps/shared",
                r#"{"key":"shared","label":"Shared","phase":"green","guidance":"from shared"}"#,
            )],
        );
        assert_eq!(template.id, "custom");
        assert_eq!(template.steps.len(), 1);
        assert_eq!(template.steps[0].key, "shared");
        assert_eq!(template.steps[0].guidance.as_deref(), Some("from shared"));
    }
}
