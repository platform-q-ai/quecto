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
            "adversarial-review",
            include_str!("../../../../workflows/adversarial-review.json"),
        ),
        ("bugfix", include_str!("../../../../workflows/bugfix.json")),
        ("chore", include_str!("../../../../workflows/chore.json")),
        (
            "flake-hunt",
            include_str!("../../../../workflows/flake-hunt.json"),
        ),
        (
            "investigate",
            include_str!("../../../../workflows/investigate.json"),
        ),
        ("plan", include_str!("../../../../workflows/plan.json")),
        ("plan2", include_str!("../../../../workflows/plan2.json")),
        ("prd", include_str!("../../../../workflows/prd.json")),
        (
            "refactor",
            include_str!("../../../../workflows/refactor.json"),
        ),
        ("remove", include_str!("../../../../workflows/remove.json")),
    ];
    /// The canonical shared-step files, keyed by their reference path
    /// (relative to the workflow dir, `.json` extension omitted).
    const STEP_FILES: &[(&str, &str)] = &[
        (
            "steps/shared/follow_ups",
            include_str!("../../../../workflows/steps/shared/follow_ups.json"),
        ),
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
        (
            "steps/shared/write_scenarios",
            include_str!("../../../../workflows/steps/shared/write_scenarios.json"),
        ),
        (
            "steps/shared/write_step_tests",
            include_str!("../../../../workflows/steps/shared/write_step_tests.json"),
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
        let reference = match entry {
            serde_json::Value::String(reference) => Some(reference.clone()),
            serde_json::Value::Object(object) => object
                .get("ref")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            _ => None,
        };
        let Some(reference) = reference else { continue };
        let resolved = step_files
            .iter()
            .find(|(path, _)| *path == reference || format!("{path}.json") == reference)
            .map(|(_, step)| step)
            .unwrap_or_else(|| {
                panic!("embedded template `{id}` references unembedded step `{reference}`")
            });
        let mut resolved_value: serde_json::Value = serde_json::from_str(resolved)
            .unwrap_or_else(|e| panic!("embedded shared step `{reference}` must parse: {e}"));
        if let (Some(resolved_object), serde_json::Value::Object(overrides)) =
            (resolved_value.as_object_mut(), &*entry)
        {
            for key in ["key", "label", "phase", "guidance"] {
                if let Some(value) = overrides.get(key) {
                    resolved_object.insert(key.to_owned(), value.clone());
                }
            }
        }
        *entry = resolved_value;
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
#[path = "templates_cov_tests.rs"]
mod cov_tests;
#[cfg(test)]
#[path = "templates_tests.rs"]
mod tests;

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
