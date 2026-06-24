export const meta = {
  name: 'feature',
  description: 'Quecto feature workflow: hooks → RED (BDD/TDD) → BDD review → GREEN → push gate → PR → parallel reviewers → fix → merge.',
  whenToUse: 'Use for all Quecto development work in this repository. Pass the issue/task description as args.',
  phases: [
    { title: 'Setup', detail: 'Install/verify local quality hooks' },
    { title: 'RED', detail: 'Update scenarios, write tests, confirm they fail' },
    { title: 'BDD Review', detail: 'Sub-agent reviews BDD feature/step/unit tests' },
    { title: 'GREEN', detail: 'Implement, refactor, verify tests pass' },
    { title: 'Ship', detail: 'Commit, push (full gate), open PR' },
    { title: 'PR Review', detail: 'Parallel Architecture/Security/Performance reviewers' },
    { title: 'Fix Reviews', detail: 'Triage findings, fix, push, resolve threads' },
    { title: 'Merge', detail: 'Confirm gate, merge, sync local master, cleanup' },
  ],
}

// The task/issue description for this feature run.
const TASK = args ? (typeof args === 'string' ? args : JSON.stringify(args)) : 'the assigned issue'

// ── Setup ────────────────────────────────────────────────────────────────
phase('Setup')
await agent(
  `Quecto feature workflow for: ${TASK}\n\n` +
  `STEP — Install/check local quality hooks. Run scripts/install-hooks.sh, then verify pre-commit, ` +
  `pre-push, and the git --no-verify wrapper are installed/active before any code is edited. ` +
  `Never bypass hooks with --no-verify. Report hook status.`,
  { label: 'hooks', phase: 'Setup' }
)

// ── RED ──────────────────────────────────────────────────────────────────
phase('RED')
await agent(
  `Task: ${TASK}\n\n` +
  `STEP — Update Scenarios / add features (RED phase, part 1). Update BDD feature files and ` +
  `task-facing scenarios FIRST, and identify explicit, checkable acceptance criteria for the change.\n` +
  `STEP — Write/update unit tests. Run a quick targeted smoke check to confirm they compile; the full ` +
  `suite and coverage run on push.\n` +
  `STEP — Ensure new/modified tests FAIL (RED). Run only the new/modified tests to confirm they fail ` +
  `before any implementation. Report the failing test names and the acceptance criteria.`,
  { label: 'red', phase: 'RED' }
)

// ── BDD Review (independent sub-agent) ─────────────────────────────────────
phase('BDD Review')
const bddReview = await agent(
  `You are an independent BDD/test-quality reviewer for the Quecto repo. Task under review: ${TASK}\n\n` +
  `Run your OWN independent review on the single dimension 'BDD/test quality'. Read the changed BDD ` +
  `feature files, step tests, and unit tests (use git diff to find them). Verify scenarios follow BDD ` +
  `best practice: clear Given-When-Then, explicit and testable acceptance criteria, one logical scenario ` +
  `per feature, NO implementation details in scenario steps, appropriate step abstraction/reusability. ` +
  `Verify unit tests are behavioural, well-named, focused, and assert the right things. Be skeptical — ` +
  `report ONLY real issues. Do NOT modify code. Return a report with, per finding: file:line, severity, ` +
  `the problem, and a concrete fix.`,
  { label: 'bdd-review', phase: 'BDD Review' }
)
log('BDD review complete — address valid findings before GREEN.')

// ── GREEN ──────────────────────────────────────────────────────────────────
phase('GREEN')
await agent(
  `Task: ${TASK}\n\n` +
  `First, fix any valid BDD/test-quality concerns from this review:\n${bddReview}\n\n` +
  `STEP — Implement code (GREEN). Write the code needed to satisfy the failing tests; implement it in ` +
  `full regardless of size.\n` +
  `STEP — Refactor. Tidy only what this change touches (naming, duplication, clarity); minimal, no ` +
  `speculative abstraction.\n` +
  `STEP — Ensure tests still pass. Re-run the targeted tests and confirm GREEN. Respect the file-size ` +
  `cap and strict clippy before pushing.`,
  { label: 'green', phase: 'GREEN' }
)

// ── Ship: commit, push (full gate), PR ─────────────────────────────────────
phase('Ship')
const pr = await agent(
  `Task: ${TASK}\n\n` +
  `STEP — Commit. If on the default branch (master), create a feature branch first. Write a clear, ` +
  `descriptive commit message with any required commit trailers. GUARD: hook setup and RED/GREEN must be ` +
  `done first.\n` +
  `STEP — Push. This triggers the full local gate: fmt, strict clippy, unit/architecture/contracts/` +
  `repo_docs, the 24-shard non-real BDD suite, region coverage at/above threshold (quecto and quecto-tui), ` +
  `machete, deny, and the real-LLM e2e suite (~140s). Fix every failure; never use --no-verify.\n` +
  `STEP — Create PR. Open the PR against master with gh, clear title and a body summarizing the change. ` +
  `Do not claim Claude co-authorship. Return the PR number and head commit SHA.`,
  { label: 'commit-push-pr', phase: 'Ship' }
)

// ── PR Review: parallel reviewers posting inline comments ──────────────────
phase('PR Review')
const DIMENSIONS = ['Architecture', 'Security', 'Performance']
const reviews = await parallel(DIMENSIONS.map(dim => () => agent(
  `You are an independent ${dim} reviewer for this Quecto PR. PR/context: ${pr}\n\n` +
  `Run your OWN independent review on the single dimension '${dim}'. Read the diff with ` +
  `gh pr diff <PR>. Be skeptical — report ONLY real issues. Do NOT modify code. ` +
  `Post findings as INLINE review comments on the PR via the GitHub GraphQL API (gh api graphql): ` +
  `fetch the PR node id and head SHA with gh pr view <PR> --json id,headRefOid, then submit one review ` +
  `carrying inline comments via the addPullRequestReview mutation (event COMMENT, comments array of ` +
  `path/line/body anchored to the head commit), or addPullRequestReviewThread per finding. If a line ` +
  `anchor is rejected (line outside diff, or PR already merged), fall back to a review comment that still ` +
  `cites file:line. Each finding: file:line, severity, the problem, a concrete fix. Return a summary of ` +
  `posted findings.`,
  { label: `review:${dim}`, phase: 'PR Review' }
)))

// ── Fix reviews, push fixes, resolve threads ───────────────────────────────
phase('Fix Reviews')
await agent(
  `Task: ${TASK}. PR: ${pr}\n\n` +
  `Reviewer findings posted inline:\n${reviews.filter(Boolean).join('\n\n---\n\n')}\n\n` +
  `STEP — Fix all valid review concerns. Triage each inline finding; confirm it is genuinely valid before ` +
  `changing anything (reviewers can be wrong). Fix forward in the same branch. Track accepted vs declined.\n` +
  `STEP — Push changes. The full pre-push gate runs again; wait for it to pass.\n` +
  `STEP — Reply to EVERY review comment and resolve threads. For accepted findings note the fix and commit; ` +
  `for declined ones explain why. Resolve each thread with the GraphQL resolveReviewThread mutation ` +
  `(thread ids from the PR reviewThreads connection).`,
  { label: 'fix-resolve', phase: 'Fix Reviews' }
)

// ── Merge, sync, cleanup ───────────────────────────────────────────────────
phase('Merge')
const result = await agent(
  `Task: ${TASK}. PR: ${pr}\n\n` +
  `STEP — Confirm the pre-push gate passed in full (coverage threshold, real-LLM, machete, deny) and the ` +
  `CI Smoke Test is green. GUARD: review and gate must be done before merging.\n` +
  `STEP — Merge with: gh pr merge <PR> --merge --auto --delete-branch (auto-merge waits for the required ` +
  `Smoke Test). The default branch is protected with enforce_admins; do not force or bypass.\n` +
  `STEP — Move to local master: git checkout master && git pull --ff-only to sync the merge.\n` +
  `Return the final merge status.`,
  { label: 'merge-sync', phase: 'Merge' }
)

log('Feature workflow complete.')
return { pr, result }
