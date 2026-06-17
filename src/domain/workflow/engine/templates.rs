use crate::domain::workflow::{WorkflowGuardRule, WorkflowTemplate, WorkflowTemplateStep};

pub fn default_templates() -> Vec<WorkflowTemplate> {
    vec![
        WorkflowTemplate {
            id: "feature".into(),
            label: "Feature".into(),
            description: "New capability with local hook verification, BDD/TDD, code review, and merge.".into(),
            when_to_use: Some("Use for any new user-facing or system-facing behavior, new commands, new tools, or substantial extensions.".into()),
            steps: vec![
                WorkflowTemplateStep {
                    key: "hooks".into(),
                    label: "Install/check local quality hooks".into(),
                    phase: "setup".into(),
                    guidance: Some("Run scripts/install-hooks.sh, then verify pre-commit, pre-push, pre-merge-commit, and the git --no-verify wrapper are installed/active before editing code.".into()),
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
                    guidance: Some("Use the subagent tool in parallel mode to dispatch architecture-reviewer, security-reviewer, and performance-reviewer.".into()),
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
                    label: "Run pre-merge hooks (real-LLM, machete, deny)".into(),
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
                    message: "Complete code review and pre-merge validation before merging.".into(),
                },
            ],
        },
        WorkflowTemplate {
            id: "fix".into(),
            label: "Fix".into(),
            description: "Bug fix or regression correction.".into(),
            when_to_use: Some(
                "Use when behavior is broken and needs reproduction plus repair.".into(),
            ),
            steps: vec![
                WorkflowTemplateStep {
                    key: "hooks".into(),
                    label: "Install/check local quality hooks".into(),
                    phase: "setup".into(),
                    guidance: Some("Run scripts/install-hooks.sh, then verify pre-commit, pre-push, pre-merge-commit, and the git --no-verify wrapper are installed/active before editing code.".into()),
                },
                WorkflowTemplateStep {
                    key: "repro".into(),
                    label: "Capture reproduction / failing scenario".into(),
                    phase: "red".into(),
                    guidance: Some(
                        "Start by reproducing the bug in a scenario or focused test.".into(),
                    ),
                },
                WorkflowTemplateStep {
                    key: "tests".into(),
                    label: "Write/update regression tests".into(),
                    phase: "red".into(),
                    guidance: None,
                },
                WorkflowTemplateStep {
                    key: "red".into(),
                    label: "Ensure regression test fails (RED)".into(),
                    phase: "red".into(),
                    guidance: None,
                },
                WorkflowTemplateStep {
                    key: "green".into(),
                    label: "Implement fix (GREEN)".into(),
                    phase: "green".into(),
                    guidance: None,
                },
                WorkflowTemplateStep {
                    key: "verify".into(),
                    label: "Verify regression is fixed".into(),
                    phase: "green".into(),
                    guidance: None,
                },
                WorkflowTemplateStep {
                    key: "commit".into(),
                    label: "Commit changes".into(),
                    phase: "ci_cd".into(),
                    guidance: None,
                },
            ],
            guards: vec![WorkflowGuardRule {
                commands: vec!["git commit".into(), "git push".into()],
                before_step_key: "commit".into(),
                message: "Complete reproduction, fix, and verification before commit/push.".into(),
            }],
        },
        WorkflowTemplate {
            id: "refactor".into(),
            label: "Refactor".into(),
            description: "Internal cleanup without intended behavioral change.".into(),
            when_to_use: Some(
                "Use for structural cleanup, performance, or design improvement.".into(),
            ),
            steps: vec![
                WorkflowTemplateStep {
                    key: "hooks".into(),
                    label: "Install/check local quality hooks".into(),
                    phase: "setup".into(),
                    guidance: Some("Run scripts/install-hooks.sh, then verify pre-commit, pre-push, pre-merge-commit, and the git --no-verify wrapper are installed/active before editing code.".into()),
                },
                WorkflowTemplateStep {
                    key: "safety".into(),
                    label: "Define safety net tests".into(),
                    phase: "red".into(),
                    guidance: Some(
                        "Identify the tests that must stay green throughout the refactor.".into(),
                    ),
                },
                WorkflowTemplateStep {
                    key: "baseline".into(),
                    label: "Verify baseline behavior".into(),
                    phase: "green".into(),
                    guidance: None,
                },
                WorkflowTemplateStep {
                    key: "refactor".into(),
                    label: "Refactor internals".into(),
                    phase: "refactor".into(),
                    guidance: None,
                },
                WorkflowTemplateStep {
                    key: "verify".into(),
                    label: "Re-run safety net".into(),
                    phase: "green".into(),
                    guidance: None,
                },
                WorkflowTemplateStep {
                    key: "commit".into(),
                    label: "Commit changes".into(),
                    phase: "ci_cd".into(),
                    guidance: None,
                },
            ],
            guards: vec![WorkflowGuardRule {
                commands: vec!["git commit".into()],
                before_step_key: "commit".into(),
                message: "Complete the refactor safety net before commit.".into(),
            }],
        },
        WorkflowTemplate {
            id: "chore".into(),
            label: "Chore".into(),
            description: "Routine maintenance or configuration work.".into(),
            when_to_use: Some("Use for docs, dependency, CI, or maintenance tasks.".into()),
            steps: vec![
                WorkflowTemplateStep {
                    key: "hooks".into(),
                    label: "Install/check local quality hooks".into(),
                    phase: "setup".into(),
                    guidance: Some("Run scripts/install-hooks.sh, then verify pre-commit, pre-push, pre-merge-commit, and the git --no-verify wrapper are installed/active before editing code.".into()),
                },
                WorkflowTemplateStep {
                    key: "scope".into(),
                    label: "Clarify the maintenance task".into(),
                    phase: "red".into(),
                    guidance: Some("Restate the task and identify any required validation.".into()),
                },
                WorkflowTemplateStep {
                    key: "change".into(),
                    label: "Apply the maintenance change".into(),
                    phase: "green".into(),
                    guidance: None,
                },
                WorkflowTemplateStep {
                    key: "verify".into(),
                    label: "Verify the change".into(),
                    phase: "green".into(),
                    guidance: None,
                },
                WorkflowTemplateStep {
                    key: "commit".into(),
                    label: "Commit changes".into(),
                    phase: "ci_cd".into(),
                    guidance: None,
                },
            ],
            guards: vec![WorkflowGuardRule {
                commands: vec!["git commit".into()],
                before_step_key: "commit".into(),
                message: "Verify the maintenance change before commit.".into(),
            }],
        },
    ]
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
