use crate::domain::workflow::WorkflowTemplate;

/// The built-in workflow templates.
///
/// These are defined ONCE in the crate-root `workflow-config.json` — the
/// guard-tested canonical spec — and embedded here at compile time, so the
/// RUNNING workflow is byte-for-byte the reviewed spec.
///
/// Previously the feature template was hand-maintained in this file and silently
/// drifted from `workflow-config.json`: stale gate facts (`Smoke Test`,
/// `QUECTO_SKIP_REAL_LLM`), missing steps (`version_bump`), and a reviewer
/// read-only instruction that lived only in a `shared_guidance` field serde
/// never deserialized — so none of it reached a running agent. Parsing the
/// single source removes that entire class of drift bug.
///
/// `include_str!` + `serde_json::from_str` is pure compile-time embedding plus
/// in-memory parsing (no filesystem or network access at runtime).
pub fn default_templates() -> Vec<WorkflowTemplate> {
    const CONFIG_JSON: &str = include_str!("../../../../workflow-config.json");

    #[derive(serde::Deserialize)]
    struct Root {
        workflow: WorkflowSection,
    }
    #[derive(serde::Deserialize)]
    struct WorkflowSection {
        templates: Vec<WorkflowTemplate>,
    }

    serde_json::from_str::<Root>(CONFIG_JSON)
        .expect("embedded workflow-config.json must deserialize into workflow templates")
        .workflow
        .templates
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
            .expect("the embedded config must define a `feature` template")
    }

    #[test]
    fn embedded_config_parses_into_a_feature_template() {
        // The whole point of parsing `workflow-config.json` at compile time: the
        // RUNTIME template is the guard-tested spec, so it cannot drift.
        let f = feature();
        assert!(!f.steps.is_empty());
        assert!(f.steps.iter().any(|s| s.key == "bdd_review"));
        assert!(f.steps.iter().any(|s| s.key == "reviewers"));
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
