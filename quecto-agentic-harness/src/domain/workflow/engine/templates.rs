use crate::domain::workflow::{WorkflowTemplate, WorkflowTemplateStep};

/// Built-in generic workflow templates shipped with the binary.
///
/// Runtime discovery still takes precedence over these defaults: configured
/// `workflow.dir`, repo-local `.quecto/workflows`, global workflow dirs, and
/// inline `workflow.templates` are resolved before the engine falls back to this
/// library. Keep these templates provider-neutral and project-neutral so clean
/// public checkouts have useful workflows without inheriting this repository's
/// own process assumptions.
pub fn default_templates() -> Vec<WorkflowTemplate> {
    vec![
        template(
            "investigate",
            "Investigate",
            "Read-only diagnosis that gathers evidence, identifies root cause or trade-offs, and reports a cited conclusion.",
            "Use when the task is to understand, triage, or explain something without changing files.",
            &[
                (
                    "scope",
                    "Define the question",
                    "setup",
                    "Restate the question, constraints, and what evidence would answer it. Do not edit files.",
                ),
                (
                    "inspect",
                    "Inspect evidence",
                    "analysis",
                    "Read relevant code, docs, logs, or configuration. Prefer primary sources and record file paths or commands used.",
                ),
                (
                    "verify",
                    "Challenge the conclusion",
                    "review",
                    "Look for contradictory evidence and alternative explanations before settling on an answer.",
                ),
                (
                    "report",
                    "Report findings",
                    "handoff",
                    "Summarize the conclusion, evidence, uncertainty, and suggested next steps. Confirm no files were modified.",
                ),
            ],
        ),
        template(
            "chore",
            "Chore",
            "Small maintenance workflow for docs, configuration, tooling, or other low-risk repository upkeep.",
            "Use for maintenance that should not intentionally change product behavior.",
            &[
                (
                    "scope",
                    "Scope the chore",
                    "setup",
                    "Restate the requested maintenance, define done, and identify the files expected to change.",
                ),
                (
                    "change",
                    "Make the minimal change",
                    "green",
                    "Apply the smallest repo-local edit that satisfies the scope. Avoid unrelated cleanup.",
                ),
                (
                    "check",
                    "Run relevant checks",
                    "verify",
                    "Run formatting, linting, tests, or documentation checks appropriate to the changed files.",
                ),
                (
                    "review",
                    "Review the diff",
                    "review",
                    "Inspect the final diff for accidental behavior changes, secrets, generated noise, and scope creep.",
                ),
                (
                    "handoff",
                    "Handoff",
                    "handoff",
                    "Summarize what changed, checks run, and any follow-up risks or skipped validation.",
                ),
            ],
        ),
        template(
            "bugfix",
            "Bugfix",
            "Reproduce a wrong behavior, fix the smallest cause, and prove the regression is covered.",
            "Use when existing observable behavior is incorrect and should be corrected.",
            &[
                (
                    "reproduce",
                    "Reproduce the failure",
                    "red",
                    "Capture the wrong behavior with a failing test, fixture, command, or clear manual reproduction.",
                ),
                (
                    "diagnose",
                    "Diagnose root cause",
                    "analysis",
                    "Trace the failure to the smallest responsible code path and check for related cases.",
                ),
                (
                    "fix",
                    "Implement the fix",
                    "green",
                    "Make the minimal code change that addresses the root cause while preserving intended behavior.",
                ),
                (
                    "regression",
                    "Prove regression coverage",
                    "verify",
                    "Run the reproduction and relevant surrounding tests to show the fix holds.",
                ),
                (
                    "handoff",
                    "Handoff",
                    "handoff",
                    "Summarize the defect, fix, validation, and remaining risk.",
                ),
            ],
        ),
        template(
            "feature",
            "Feature",
            "Implement a planned behavior change with explicit acceptance criteria and verification.",
            "Use when adding or changing user-visible behavior from an agreed request or plan.",
            &[
                (
                    "intake",
                    "Confirm acceptance criteria",
                    "setup",
                    "Restate the desired behavior, constraints, and how completion will be verified.",
                ),
                (
                    "test_design",
                    "Design verification",
                    "red",
                    "Decide which tests, examples, or checks will prove the behavior before implementation.",
                ),
                (
                    "implement",
                    "Implement the slice",
                    "green",
                    "Build the smallest coherent slice that satisfies the agreed criteria.",
                ),
                (
                    "refine",
                    "Refine safely",
                    "refactor",
                    "Improve clarity or structure only while keeping the new verification green.",
                ),
                (
                    "validate",
                    "Validate",
                    "verify",
                    "Run targeted and relevant broader checks; compare results to acceptance criteria.",
                ),
                (
                    "handoff",
                    "Handoff",
                    "handoff",
                    "Summarize behavior delivered, validation, and follow-up work.",
                ),
            ],
        ),
        template(
            "refactor",
            "Refactor",
            "Behavior-preserving restructure backed by characterization and parity checks.",
            "Use when changing structure, names, or organization without intended behavior change.",
            &[
                (
                    "scope",
                    "Define invariants",
                    "setup",
                    "State what must remain unchanged and what structure is allowed to change.",
                ),
                (
                    "characterize",
                    "Characterize current behavior",
                    "verify",
                    "Run or add checks that would fail if behavior changed accidentally.",
                ),
                (
                    "refactor",
                    "Refactor incrementally",
                    "refactor",
                    "Make small structural changes, keeping characterization checks passing.",
                ),
                (
                    "parity",
                    "Prove parity",
                    "verify",
                    "Run relevant tests and inspect the diff for unintended behavior changes.",
                ),
                (
                    "handoff",
                    "Handoff",
                    "handoff",
                    "Summarize the restructure, parity evidence, and any residual risk.",
                ),
            ],
        ),
    ]
}

fn template(
    id: &str,
    label: &str,
    description: &str,
    when_to_use: &str,
    steps: &[(&str, &str, &str, &str)],
) -> WorkflowTemplate {
    WorkflowTemplate {
        id: id.into(),
        label: label.into(),
        description: description.into(),
        when_to_use: Some(when_to_use.into()),
        steps: steps
            .iter()
            .map(|(key, label, phase, guidance)| WorkflowTemplateStep {
                key: (*key).into(),
                label: (*label).into(),
                phase: (*phase).into(),
                guidance: Some((*guidance).into()),
            })
            .collect(),
        guards: Vec::new(),
    }
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

#[cfg(test)]
#[path = "templates_tests.rs"]
mod tests;
