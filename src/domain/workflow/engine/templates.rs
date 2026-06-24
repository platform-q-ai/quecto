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
                guidance: Some("Run scripts/install-hooks.sh, then verify pre-commit, pre-push, and the git --no-verify wrapper are installed/active before editing code. Never bypass hooks with --no-verify.".into()),
            },
            WorkflowTemplateStep {
                key: "scenarios".into(),
                label: "Update Scenarios / Add new features".into(),
                phase: "red".into(),
                guidance: Some("Update BDD feature files and task-facing scenarios first, and identify explicit, checkable acceptance criteria for the change.".into()),
            },
            WorkflowTemplateStep {
                key: "tests".into(),
                label: "Write/update unit tests (run a quick smoke check; full suite runs on push)".into(),
                phase: "red".into(),
                guidance: Some("Write or update the unit tests for the change. Run a quick targeted smoke check to confirm they compile; the full suite and coverage run on push.".into()),
            },
            WorkflowTemplateStep {
                key: "red".into(),
                label: "Ensure new/modified tests FAIL (RED) — quick targeted run only, not full suite".into(),
                phase: "red".into(),
                guidance: Some("Run only the new/modified tests to confirm they fail before any implementation.".into()),
            },
            WorkflowTemplateStep {
                key: "bdd_review".into(),
                label: "Despatch BDD sub-agent to review BDD feature, step tests and unit tests".into(),
                phase: "review".into(),
                guidance: Some("Launch a sub-agent with `spawn` and bind it to a dedicated BDD review workflow by passing `workflow_spec` with a `template` (e.g., `bdd-review`) containing steps: read the changed BDD feature files, step tests, and unit tests; verify they follow BDD best practice; return a report with file:line findings. Use `agent_id='bdd-review'`, then use `agent_cmd` with `command: 'await'` (and a suitable `timeout`) to wait for it to finish, followed by `get_messages_tail` to read its findings. The reviewer runs its OWN independent review (NOT this feature workflow): give it the issue details, the changed files, and the single review dimension 'BDD/test quality'. It must verify that scenarios follow BDD best practice (clear Given-When-Then structure, explicit and testable acceptance criteria, one logical scenario per feature, no implementation details in scenario steps, appropriate step abstraction and reusability), and that unit tests are behavioural, well-named, focused, and assert the right things. The reviewer must be skeptical (report only real issues), must NOT modify code, and must return a report with file:line findings, severity, the problem, and a concrete fix. Fix any valid BDD/test-quality concerns before moving to the GREEN step.".into()),
            },
            WorkflowTemplateStep {
                key: "green".into(),
                label: "Implement code (GREEN)".into(),
                phase: "green".into(),
                guidance: Some("Write the code needed to satisfy the failing tests. Do NOT worry about the size of a change — implement it in full.".into()),
            },
            WorkflowTemplateStep {
                key: "refactor".into(),
                label: "Refactor".into(),
                phase: "refactor".into(),
                guidance: Some("Tidy only what this change touches — naming, duplication, clarity. Keep it minimal; no speculative abstraction or unrelated cleanup.".into()),
            },
            WorkflowTemplateStep {
                key: "verify".into(),
                label: "Ensure tests still pass".into(),
                phase: "green".into(),
                guidance: Some("Re-run the targeted tests and confirm GREEN. Respect the file-size cap and strict clippy before pushing.".into()),
            },
            WorkflowTemplateStep {
                key: "commit".into(),
                label: "Commit".into(),
                phase: "ci_cd".into(),
                guidance: Some("If on the default branch, create a feature branch first. Write a clear, descriptive commit message and include any commit trailers your harness requires.".into()),
            },
            WorkflowTemplateStep {
                key: "push".into(),
                label: "Push (pre-push hook will run tests and linting)".into(),
                phase: "ci_cd".into(),
                guidance: Some("Push triggers the full local gate: fmt, strict clippy, unit/architecture/contracts/repo_docs, the 24-shard non-real BDD suite, region coverage at or above the project threshold (quecto and quecto-tui), machete, deny, and the real-LLM e2e suite (~140s; set QUECTO_SKIP_REAL_LLM=1 only for throwaway WIP pushes). Fix every failure; never use --no-verify.".into()),
            },
            WorkflowTemplateStep {
                key: "pr".into(),
                label: "Create PR".into(),
                phase: "ci_cd".into(),
                guidance: Some("Open the PR against the default branch with gh, with a clear title and a body that summarizes the change. The required Smoke Test check runs in CI.".into()),
            },
            WorkflowTemplateStep {
                key: "reviewers".into(),
                label: "Despatch sub agents in parallel as reviewers (Architecture, Security and Performance)".into(),
                phase: "review".into(),
                guidance: Some("Dispatch the reviewers as parallel sub-agents in a SINGLE batch (one message, multiple subagent calls) so they run concurrently. Bind each reviewer to a dedicated review workflow by passing `workflow_spec` with a `template` (e.g., `review-pr`) containing steps: fetch the PR diff, analyze the assigned dimension, and post findings as inline PR comments. At minimum dispatch Architecture, Security, and Performance; add Correctness and Test-quality for larger changes. Each reviewer runs its OWN independent review (NOT this feature workflow): give it only the PR number, the head commit SHA, its single review dimension, and the file scope; it reads the diff with gh pr diff <PR>, forms findings, and posts them. Reviewers must be skeptical (report only real issues) and must NOT modify code. Every reviewer MUST post findings as INLINE review comments on the PR via the GitHub GraphQL API (gh api graphql) — not a summary, not just a returned report: fetch the PR node id and head SHA with gh pr view <PR> --json id,headRefOid, then submit one review carrying inline comments using the addPullRequestReview mutation (event COMMENT, with a comments array of path/line/body entries anchored to the head commit) or addPullRequestReviewThread per finding. If a line anchor is rejected (line outside the diff, or the PR is already merged), fall back to a review comment that still cites file:line for every finding — inline is the default. Each finding states file:line, severity, the problem, and a concrete fix. After spawning, await each reviewer with `agent_cmd` `command: 'await'` (and a suitable `timeout`) so the parent is notified when every review has finished; verify the posted findings on the PR before moving to the fix step. Record each spawned reviewer's sub-agent id for the cleanup step.".into()),
            },
            WorkflowTemplateStep {
                key: "fix_reviews".into(),
                label: "Fix all valid review concerns".into(),
                phase: "review".into(),
                guidance: Some("Triage each inline finding — confirm it is genuinely valid before changing anything (reviewers can be wrong). Fix forward in the same branch. Track which findings you accept versus decline; you reply to all of them in the resolve step.".into()),
            },
            WorkflowTemplateStep {
                key: "push_fixes".into(),
                label: "Push changes to remote".into(),
                phase: "review".into(),
                guidance: Some("Push the fixes; the full pre-push gate runs again. Wait for it to pass before resolving threads.".into()),
            },
            WorkflowTemplateStep {
                key: "resolve_threads".into(),
                label: "Reply to the reviewers comments on the PR and mark resolved (use graphql)".into(),
                phase: "review".into(),
                guidance: Some("Reply to EVERY review comment on the PR — for accepted findings note the fix and commit, for declined ones explain why — then resolve each thread with the GraphQL resolveReviewThread mutation. Thread ids come from the PR reviewThreads connection.".into()),
            },
            WorkflowTemplateStep {
                key: "pre_merge".into(),
                label: "Confirm the pre-push gate passed (real-LLM, machete, deny run on push)".into(),
                phase: "ci_cd".into(),
                guidance: Some("Confirm the latest push's pre-push gate passed in full (coverage threshold, real-LLM, machete, deny) and the CI Smoke Test is green before merging.".into()),
            },
            WorkflowTemplateStep {
                key: "merge".into(),
                label: "Merge".into(),
                phase: "ci_cd".into(),
                guidance: Some("Merge with gh pr merge <PR> --merge --auto --delete-branch (auto-merge waits for the required Smoke Test). The default branch is protected with enforce_admins; do not force or bypass.".into()),
            },
            WorkflowTemplateStep {
                key: "pull".into(),
                label: "Move to local master and pull".into(),
                phase: "ci_cd".into(),
                guidance: Some("Run git checkout master and git pull --ff-only to sync the merge locally.".into()),
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
