use crate::domain::workflow::WorkflowTemplate;

/// Built-in workflow templates.
///
/// Quecto no longer ships a public-repo workflow template library. Runtime
/// templates are discovered from configured or user-local workflow directories
/// (`workflow.dir`, `<cwd>/.quecto/workflows`, then `~/.quecto/workflows`) or
/// from inline `workflow.templates` in config. An empty config therefore means
/// an empty library, not bundled defaults.
pub fn default_templates() -> Vec<WorkflowTemplate> {
    Vec::new()
}

pub(super) fn phase_display_name(phase: &str) -> &str {
    match phase {
        "red" => "RED",
        "green" => "GREEN",
        "refactor" => "REFACTOR",
        "review" => "REVIEW",
        "blue" => "BLUE",
        "purple" => "PURPLE",
        other => other,
    }
}
