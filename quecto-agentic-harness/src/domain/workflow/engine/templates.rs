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
                    "Deliver a bounded report stating the conclusion, auditable evidence, remaining uncertainty, and suggested next steps. Confirm workspace preservation from the actions taken and available evidence. Mark this step complete only after that report has been delivered; a completion check is not a substitute for the report.",
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
                    "Run checks that address the changed artifact's intended use and material failure modes. For documentation or configuration, compare against the authoritative source and, where practical and safe, exercise the documented example or configuration and inspect its result. Use an isolated or non-destructive check when execution would create or alter artifacts; otherwise state what source comparison establishes and what remains unverified. Record results before marking this step complete.",
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
                    "Before marking this step complete, provide the handoff: summarize what changed, identify the checks actually performed and their results, and disclose remaining risks or skipped validation. Distinguish completed work from recommended next actions. If this summary has already been provided, reference it rather than repeat it.",
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
                    "Before changing the implementation, observe and record the wrong behavior with a failing test, command, fixture check, or clear manual reproduction that distinguishes the defect from setup failure. For documentation, configuration, or non-executable work, a specific source-to-artifact comparison or reproducible derivation may establish the mismatch instead. If reproduction is unavailable, record the limitation before proceeding. If implementation has already changed, disclose that chronology; retrospective checks can establish regression sensitivity but not pre-change observation. Do not undo others' work or introduce a defect to manufacture a failure.",
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
                    "Run the reproduction and relevant surrounding checks on the final artifact. Then inspect the final changes for unintended behavior, compatibility or preservation problems, and unrelated edits. Use an available diff or compare the relevant before-and-after content; version-control tooling is not required. Record the results and any unreviewed areas before completing this step.",
                ),
                (
                    "handoff",
                    "Handoff",
                    "handoff",
                    "Before marking this step complete, provide the handoff: summarize the defect, fix, checks actually run and their results, changed artifacts, and remaining risk or next action. Distinguish successful checks from unavailable or skipped validation.",
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
                    "Before implementation, choose and run a focused check that fails because the requested behavior is absent or wrong, not because setup is broken. Record the expected behavior and observed result before completing this step. Where executable RED is unsuitable, record a discriminating source, data, or artifact comparison instead. If implementation already occurred, disclose the missed ordering; later checks are not pre-change evidence.",
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
                    "Review the implemented slice against the acceptance criteria and existing invariants, including scope, compatibility, and preservation. Use an available diff or a before-and-after content comparison. If refinement is warranted, keep it small and rerun the affected verification after changes. If no refinement is warranted, state why; do not change working code merely to satisfy this step. Record the review and verification results before completing it.",
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
        template(
            "adversarial-review",
            "Adversarial review",
            "Read-only, evidence-based challenge of supplied changes, with independently validated findings and a bounded report.",
            "Use to review code or non-code changes skeptically without implementing fixes.",
            &[
                (
                    "scope",
                    "Define review boundaries",
                    "setup",
                    "Review supplied changes read-only by default. State scope, intended behavior or claims, invariants, threat assumptions, and a bounded review budget. Identify the baseline and missing context; ask for essential inputs rather than inventing them. Do not change production or the supplied artifacts, or exploit live systems.",
                ),
                (
                    "inspect",
                    "Inspect changes and context",
                    "analysis",
                    "Compare the supplied changes with available before-and-after content; no version-control system is required. Trace affected consumers, dependencies, and surrounding contracts, including documentation, configuration, data, or other non-code artifacts. Record exact locators and separate introduced problems from pre-existing conditions.",
                ),
                (
                    "challenge",
                    "Test plausible failure hypotheses",
                    "verify",
                    "Form concrete failure hypotheses from the changes and invariants; prioritize plausible impact over speculative edge cases. Actively try to falsify them with safe, bounded checks: isolated reproductions, examples, source comparisons, or reproducible derivations. Use disposable scratch artifacts only when safe and authorized; avoid destructive actions, secrets, external side effects, and live exploitation. Record expected versus observed results. If execution is unsafe or unavailable, use static evidence and disclose what remains untested.",
                ),
                (
                    "validate",
                    "Independently validate candidates",
                    "review",
                    "For each candidate, independently retrace the evidence and reproduction rather than trusting an initial claim or automated output. Seek counterevidence, intended behavior, mitigating controls, and alternative explanations. Confirm reachability and impact under stated assumptions; reject disproved claims and deduplicate related symptoms. Distinguish verified findings from unresolved hypotheses and missing evidence. No finding quota: do not invent bugs, and accept that no findings may survive.",
                ),
                (
                    "report",
                    "Deliver bounded review report",
                    "handoff",
                    "Deliver the report before marking this step complete; a completion check is not the report. Prioritize actionable findings by severity justified by impact and likelihood. For each, give exact artifact locators (path and line, section, cell, or equivalent), the violated invariant, evidence and reproduction or derivation, assumptions, and a focused remediation recommendation without applying it. Separate uncertainty and unverified concerns from findings. State reviewed scope, checks and results, counterevidence, limitations, and workspace preservation based on actions and evidence. If no findings survived, say so without claiming absence of bugs; do not require further review beyond the agreed budget.",
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
