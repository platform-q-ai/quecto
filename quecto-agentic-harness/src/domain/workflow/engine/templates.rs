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
            "prd",
            "PRD",
            "Define behavior-only product requirements without designing or implementing the solution.",
            "Use to clarify the problem and observable outcomes before execution planning.",
            &[
                (
                    "scope",
                    "Frame the product need",
                    "setup",
                    "Ground the problem, intended users, desired outcomes, and non-goals in the request and available context. Keep detail proportional to the task. Do not invent requirements or claim user approval. Writing requested documents is permitted; do not edit product code during this workflow.",
                ),
                (
                    "behavior",
                    "Describe observable behavior",
                    "analysis",
                    "Describe user-visible behavior with concrete scenarios and edge cases, including relevant failure and recovery behavior. Separate stated requirements from assumptions and open questions. Exclude architecture, technology choices, file or class layouts, and implementation task breakdowns.",
                ),
                (
                    "acceptance",
                    "Define measurable acceptance",
                    "verify",
                    "Give each outcome observable, measurable acceptance criteria and a way to assess them. Include applicable accessibility, privacy, security, and performance expectations as behavior, not solution design. Do not invent targets: mark missing thresholds or requirements as open questions and ask for clarification where they block agreement.",
                ),
                (
                    "review",
                    "Resolve gaps and check scope",
                    "review",
                    "Check scenarios and acceptance against the requested problem, users, outcomes, and non-goals. Identify contradictions, assumptions, and open questions; distinguish blocking decisions from optional refinements. Do not silently resolve uncertainty or imply agreement that has not been given.",
                ),
                (
                    "handoff",
                    "Deliver the bounded PRD",
                    "handoff",
                    "Before completing, deliver the concise behavior-only PRD in the requested document or response. Include scope, acceptance, assumptions, open questions, and explicit unresolved blockers. State its actual agreement status and the next clarification or planning handoff; stop without design, implementation planning, or product-code edits.",
                ),
            ],
        ),
        template(
            "plan",
            "Plan",
            "Turn agreed requirements and repository evidence into a verifiable execution plan without implementing it.",
            "Use when requirements are agreed and the next deliverable is an execution plan, not product changes.",
            &[
                (
                    "ground",
                    "Ground requirements and context",
                    "setup",
                    "Identify agreed requirements, acceptance criteria, constraints, and actual repository context. Inspect relevant code, tests, documentation, and existing conventions before proposing work. If agreement or essential context is missing, identify the blocker and seek clarification rather than invent requirements or claim approval. Writing requested plan documents is permitted; do not edit product code or implement the plan during this workflow.",
                ),
                (
                    "increments",
                    "Sequence verifiable increments",
                    "analysis",
                    "Describe small dependency-ordered increments grounded in the repository, each linked to requirements, prerequisites, a bounded change, and an observable completion check. Plan TDD red/green/refactor where applicable: observe a meaningful failing check, make the smallest passing change, then refactor with checks still passing. For work unsuitable for TDD, specify a suitable validation alternative. Plan these actions; do not execute implementation.",
                ),
                (
                    "risks",
                    "Expose risks and decisions",
                    "review",
                    "Identify material risks, assumptions, dependencies, and decision points before dependent increments. State what evidence or clarification resolves each blocking uncertainty. Do not silently change requirements; surface any proposed scope change for agreement and leave affected work blocked.",
                ),
                (
                    "checks",
                    "Plan checks and delivery safety",
                    "verify",
                    "Specify focused checks per increment and broader acceptance checks tied to the agreed outcomes. Include rollout and rollback or recovery steps proportional to the change; state when they are not applicable. Distinguish planned checks from evidence already obtained, and disclose unavailable checks or context.",
                ),
                (
                    "handoff",
                    "Deliver the execution plan",
                    "handoff",
                    "Before completing, deliver the bounded plan in the requested document or response: ordered increments, dependencies, checks, risks, decision points, and explicit unresolved blockers. Confirm coverage of agreed requirements without expanding scope. State what is ready, what remains blocked, and the next execution handoff. Stop without implementation or product-code edits; a plan is not evidence that its checks passed.",
                ),
            ],
        ),
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
                    "Before implementation, restate the desired behavior and constraints, and identify a focused pre-change check with its expected result that distinguishes the requested behavior from current behavior. If executable verification is unsuitable or unsafe, identify a discriminating source, data, or artifact comparison instead. Record this plan and complete intake, then write verification and obtain the pre-change evidence in Confirm RED before implementing.",
                ),
                (
                    "test_design",
                    "Write verification",
                    "red",
                    "Write targeted tests or checks for the intended behavior and agreed acceptance criteria before implementation. Make the expected result explicit and distinguish the missing behavior from existing regressions. For non-executable work, prepare a discriminating source, data, or artifact comparison. This step prepares verification; execute and record pre-change evidence separately in Confirm RED.",
                ),
                (
                    "confirm_red",
                    "Confirm RED",
                    "red",
                    "Before implementation, execute the new targeted checks and confirm they fail because the intended behavior is absent or wrong, not because setup is broken. Fix setup first; existing regression checks should remain green. If a new check already passes, reassess what behavior is missing rather than manufacture a failure. Where executable RED is unsuitable or unsafe, perform and record the prepared discriminating source, data, or artifact comparison and its expected-versus-observed result. Record the checks and evidence before completing this step. If implementation already occurred or pre-change verification is unavailable, disclose the missed chronology or limitation; later checks are not pre-change evidence.",
                ),
                (
                    "implement",
                    "Implement the slice",
                    "green",
                    "Build the smallest coherent slice that satisfies the agreed criteria. Before completing implementation and entering refactor, rerun the new targeted checks and relevant existing regression checks, and record passing results. Fix failures before proceeding; do not treat implementation alone as GREEN. For non-executable work, perform a proportional source, data, or artifact comparison against the agreed expected result and record evidence that the intended change is satisfied and relevant existing behavior is preserved. Disclose unavailable validation rather than claim it passed.",
                ),
                (
                    "refine",
                    "Refine safely",
                    "refactor",
                    "Review the implemented slice against the acceptance criteria and existing invariants, including scope, compatibility, and preservation. Use an available diff or a before-and-after content comparison. If refinement is warranted, keep it small and rerun the affected verification after changes. If no refinement is warranted, state why; do not change working code merely to satisfy this step. Record the review and verification results before completing it.",
                ),
                (
                    "adversarial_review",
                    "Adversarial review",
                    "review",
                    "Pin the complete change revision or artifact snapshot, baseline workspace state, acceptance criteria, and invariants. Accept supplied diffs with provenance limitations. Review without editing, within a proportional budget. Scale narrow finder angles across removed invariants, cross-file effects, security, performance, and test falsifiability; prefer parallel independent contexts when available. Each candidate needs concrete inputs/state leading to a wrong outcome and a precise locator. Separately attempt to REFUTE candidates with safe checks and counterevidence. Classify CONFIRMED when the triggering scenario is established, PLAUSIBLE when the mechanism is supported but the trigger is uncertain, or REFUTED with explicit counterevidence. Verifier errors or empty output leave candidates unresolved. Deduplicate the same mechanism; no finding quota. Deliver one consolidated review with evidence, severity, classifications, and limitations. Publish externally only if explicitly authorized and verify delivery. Preserve workspace state relative to the baseline, not an assumed clean workspace; inspect actual state and clean up only owned reviewer workers and scratch artifacts.",
                ),
                (
                    "fix_review_findings",
                    "Fix review findings",
                    "green",
                    "Fix confirmed, substantiated issues minimally; investigate PLAUSIBLE candidates before deciding whether a fix is warranted, never blindly fix them or invent findings. Where applicable, record meaningful regression RED before each fix, then rerun targeted and relevant regression checks and record GREEN; use proportional expected-versus-observed comparisons for non-executable work. Pin the updated snapshot and re-review fixes and affected scope using separate discovery and refutation, updating the consolidated review. Repeat within the review budget until no unresolved blocking findings remain, or explicitly report blockers without silently waiving them or claiming readiness. Errors or empty verification remain unresolved. If no substantiated issues require fixes, record no changes needed and disclose remaining uncertainty. Preserve unrelated baseline workspace changes and clean up only owned reviewer resources based on actual state. External publication requires explicit authorization and verified delivery.",
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
                    "Review supplied changes without editing them. Pin the exact target revision or artifact snapshot, baseline, acceptance criteria, invariants, and a proportional review budget. Accept supplied diffs with explicit provenance and missing-context limitations; ask for essential inputs rather than inventing them. Record workspace state relative to the baseline; it need not be clean. Do not change production or exploit live systems.",
                ),
                (
                    "inspect",
                    "Inspect changes and context",
                    "analysis",
                    "Inspect available before-and-after content and surrounding contracts, including non-code artifacts. Trace affected consumers and dependencies; distinguish introduced problems from pre-existing conditions. Record precise locators and limitations of incomplete snapshots. No particular version-control system is required.",
                ),
                (
                    "challenge",
                    "Discover candidate failures",
                    "analysis",
                    "Scale narrow finder angles to the change: removed invariants, cross-file effects, security, performance, and whether tests can falsify the claimed behavior. Use parallel independent contexts when available, not as a prerequisite. Discover candidate mechanisms separately from verification. Each candidate needs concrete inputs or state leading to a wrong outcome, a violated criterion or invariant, and a precise locator. Do not invent findings or impose a quota.",
                ),
                (
                    "validate",
                    "Attempt to refute candidates",
                    "verify",
                    "Attempt to REFUTE each candidate independently of discovery: retrace its mechanism, seek intended behavior, mitigating controls, and counterevidence. Use safe bounded reproductions or source comparisons; record expected versus observed results. Avoid destructive actions, secrets, and external side effects; use scratch artifacts only when safe and authorized. Classify CONFIRMED when the triggering scenario is established, PLAUSIBLE when the mechanism is supported but the trigger is uncertain, or REFUTED with explicit counterevidence. Verifier errors or empty output leave the candidate unresolved, not confirmed or dismissed. Deduplicate findings with the same mechanism.",
                ),
                (
                    "report",
                    "Deliver bounded review report",
                    "handoff",
                    "Deliver one consolidated review before completing: pinned scope, severity justified by impact and likelihood, precise locators, triggering inputs/state and wrong outcome, evidence, classification, counterevidence, and focused remediation without applying it. Include unresolved candidates, blockers, checks, and provenance limitations. If no findings survive, say so without claiming absence of bugs. Publish externally only if explicitly authorized and verify delivery if publishing. Preserve the workspace relative to its recorded baseline; inspect actual worker state and clean up only owned reviewer workers and scratch artifacts. Report preservation evidence and limitations; stop at the agreed budget.",
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
