use crate::domain::workflow::{WorkflowGuardRule, WorkflowTemplate, WorkflowTemplateStep};

pub fn default_templates() -> Vec<WorkflowTemplate> {
    vec![WorkflowTemplate {
        id: "feature".into(),
        label: "Feature".into(),
        description:
            "New capability with local hook verification, BDD/TDD, code review, and merge.".into(),
        when_to_use: Some("Use for all Quecto development work in this repository.".into()),
        steps: vec![
            WorkflowTemplateStep {
                key: "hooks".into(),
                label: "Install/check local quality hooks".into(),
                phase: "setup".into(),
                guidance: Some("Run scripts/install-hooks.sh, then verify pre-commit, pre-push, and the git --no-verify wrapper are installed/active before editing code.".into()),
            },
            WorkflowTemplateStep {
                key: "scenarios".into(),
                label: "Update Scenarios / Add new features".into(),
                phase: "red".into(),
                guidance: Some("Start by updating feature coverage and task-facing scenarios. Identify acceptance criteria.".into()),
            },
            WorkflowTemplateStep {
                key: "tests".into(),
                label: "Write/update unit tests (run a quick smoke check; full suite runs on push)".into(),
                phase: "red".into(),
                guidance: Some("Write or update the unit tests for the change. Run a quick targeted smoke check to confirm they compile.".into()),
            },
            WorkflowTemplateStep {
                key: "red".into(),
                label: "Ensure new/modified tests FAIL (RED) — quick targeted run only, not full suite".into(),
                phase: "red".into(),
                guidance: Some("Run only the new/modified tests to confirm they fail before any implementation.".into()),
            },
            WorkflowTemplateStep {
                key: "green".into(),
                label: "Implement code (GREEN)".into(),
                phase: "green".into(),
                guidance: Some("Write the minimum code needed to satisfy the failing tests. Do NOT worry about the size of a change — implement it in full.".into()),
            },
            WorkflowTemplateStep {
                key: "refactor".into(),
                label: "Refactor".into(),
                phase: "refactor".into(),
                guidance: None,
            },
            WorkflowTemplateStep {
                key: "verify".into(),
                label: "Ensure tests still pass".into(),
                phase: "green".into(),
                guidance: None,
            },
            WorkflowTemplateStep {
                key: "commit".into(),
                label: "Commit".into(),
                phase: "ci_cd".into(),
                guidance: None,
            },
            WorkflowTemplateStep {
                key: "push".into(),
                label: "Push (pre-push hook will run tests and linting)".into(),
                phase: "ci_cd".into(),
                guidance: None,
            },
            WorkflowTemplateStep {
                key: "pr".into(),
                label: "Create PR".into(),
                phase: "ci_cd".into(),
                guidance: None,
            },
            WorkflowTemplateStep {
                key: "reviewers".into(),
                label: "Despatch sub agents in parallel as reviewers (Architecture, Security and Performance)".into(),
                phase: "review".into(),
                guidance: Some("Use the subagent tool in parallel mode to dispatch architecture-reviewer, security-reviewer, and performance-reviewer. Start each sub agent on its OWN workflow. Instruct every reviewer to ALWAYS leave its findings as inline comments on the PR in GitHub via the GraphQL API (addPullRequestReviewThread, anchored to the specific file and line), not merely as a summary.".into()),
            },
            WorkflowTemplateStep {
                key: "fix_reviews".into(),
                label: "Fix all valid review concerns".into(),
                phase: "review".into(),
                guidance: None,
            },
            WorkflowTemplateStep {
                key: "push_fixes".into(),
                label: "Push changes to remote".into(),
                phase: "review".into(),
                guidance: None,
            },
            WorkflowTemplateStep {
                key: "resolve_threads".into(),
                label: "Reply to the reviewers comments on the PR and mark resolved (use graphql)".into(),
                phase: "review".into(),
                guidance: Some("Reply to every review comment on the PR, then resolve the threads using GraphQL mutations.".into()),
            },
            WorkflowTemplateStep {
                key: "pre_merge".into(),
                label: "Confirm the pre-push gate passed (real-LLM, machete, deny run on push)".into(),
                phase: "ci_cd".into(),
                guidance: None,
            },
            WorkflowTemplateStep {
                key: "merge".into(),
                label: "Merge".into(),
                phase: "ci_cd".into(),
                guidance: None,
            },
            WorkflowTemplateStep {
                key: "pull".into(),
                label: "Move to local master and pull".into(),
                phase: "ci_cd".into(),
                guidance: None,
            },
            WorkflowTemplateStep {
                key: "cleanup".into(),
                label: "Clean up sub agents".into(),
                phase: "ci_cd".into(),
                guidance: Some("Terminate any sub agents spawned during this workflow now that it is complete (use agent_cmd to abort them, or get_subagents then kill each) so no orphaned sub agents remain.".into()),
            },
        ],
        guards: vec![
            WorkflowGuardRule {
                commands: vec!["git commit".into(), "git push".into()],
                before_step_key: "commit".into(),
                message: "Complete hook setup and RED/GREEN work before committing.".into(),
            },
            WorkflowGuardRule {
                commands: vec!["git merge".into(), "gh pr merge".into()],
                before_step_key: "merge".into(),
                message: "Complete code review and verify the pre-push gate passed before merging.".into(),
            },
        ],
    }]
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
