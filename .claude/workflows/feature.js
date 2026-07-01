export const meta = {
  name: 'feature',
  description: 'Quecto feature workflow: hooks → RED (BDD/TDD) → BDD review → GREEN → push gate → PR → parallel reviewers → fix → conformance → report PR (no merge).',
  whenToUse: 'Use for all Quecto development work in this repository. Pass the issue/task description as args.',
  phases: [
    { title: 'Setup', detail: 'Install/verify local quality hooks' },
    { title: 'RED', detail: 'Update scenarios, write tests, confirm they fail' },
    { title: 'BDD Review', detail: 'Sub-agent reviews BDD feature/step/unit tests' },
    { title: 'GREEN', detail: 'Implement, refactor, verify tests pass' },
    { title: 'Version', detail: 'Bump semver for every changed crate + sync version docs' },
    { title: 'Ship', detail: 'Commit, push (full gate), open PR' },
    { title: 'PR Review', detail: 'Parallel Architecture/Security/Performance reviewers' },
    { title: 'Fix Reviews', detail: 'Triage findings, fix, push, resolve threads' },
    { title: 'Conformance', detail: 'Systematic check the PR meets every issue acceptance criterion before merge' },
    { title: 'Report', detail: 'Confirm gate/checks, report PR ready, cleanup (do not merge)' },
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
  `task-facing scenarios FIRST after 'gh issue view <N> --json title,body,comments', and identify explicit, checkable acceptance criteria for the change.\n` +
  `STEP — Write/update unit tests. Use 'cargo test -p quecto --lib <name_substring>' or 'cargo test -p quecto-tui --lib <name_substring>' (never '-p quecto-agentic-harness'). Run a quick targeted smoke check to confirm they compile; the full ` +
  `suite and coverage run on push.\n` +
  `STEP — Ensure new/modified tests FAIL (RED). Run only the new/modified targeted test to confirm it fails ` +
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
  `report ONLY real issues. You are launched read-only (the write and edit tools are disabled — read_only), ` +
  `so you physically cannot create or modify repo files; do NOT attempt to. Return a report with, per finding: ` +
  `file:line, severity, the problem, and a concrete fix.`,
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
  `cap and strict clippy before pushing. Do not manually re-run the whole suite before commit; push/pre-push does that.`,
  { label: 'green', phase: 'GREEN' }
)

// ── Version bump: semver for every changed crate + version docs ────────────
phase('Version')
await agent(
  `Task: ${TASK}\n\n` +
  `STEP — Bump semver for every crate this change touches. Determine which crates have modified source ` +
  `(e.g. 'git diff --name-only master...HEAD' plus unstaged changes) and, for EACH changed crate, bump ` +
  `its version in that crate's Cargo.toml — patch by default, minor for a notable feature. Do NOT bump ` +
  `crates you did not change. Keep version docs in lockstep: for the 'quecto' kernel update README.md ` +
  `'Current version: **x.y.z**' and the matching assertion in ` +
  `quecto-agentic-harness/tests/features/repo_docs.feature so both equal the new version; for any other ` +
  `crate, update whatever doc/test asserts its version. Report the old→new version for each bumped crate.`,
  { label: 'version-bump', phase: 'Version' }
)

// ── Ship: commit, push (full gate), PR ─────────────────────────────────────
phase('Ship')
const pr = await agent(
  `Task: ${TASK}\n\n` +
  `STEP — Commit. If on the default branch (master), create a feature branch first. Stage only intended files. Remember git commit pre-commit does not run unit/BDD tests. Write a clear, ` +
  `descriptive commit message with any required commit trailers. GUARD: hook setup and RED/GREEN must be ` +
  `done first.\n` +
  `STEP — Push. This triggers the full local gate (or run scripts/pre-push.sh without pushing): fmt, strict clippy, unit/architecture/contracts/` +
  `repo_docs, the 24-shard non-real BDD suite, region coverage at/above threshold (quecto and quecto-tui), ` +
  `machete, deny, and the zero-cost mocked @mock-llm e2e lane (the live suite is opt-in via ` +
  `QUECTO_RUN_REAL_LLM=1). Fix every failure; never use --no-verify — a bypassed gate does NOT count as ` +
  `passing. If only the load-flaky find.feature scenario fails, re-run the shard.\n` +
  `STEP — Create PR. Open the PR against master with gh, clear title and a body summarizing the change. ` +
  `Do not claim Claude co-authorship. Return the PR number and head commit SHA.`,
  { label: 'commit-push-pr', phase: 'Ship' }
)

// ── PR Review: parallel reviewers posting inline comments ──────────────────
phase('PR Review')
// Always dispatch the full default dimension set in one parallel batch (#845):
// Architecture/Security/Performance plus Correctness, Conformance-to-AC and
// Test-quality — the trio alone has predictable blind spots (silent-wrong
// answers, unmet ACs incl. docs, hollow tests).
const DIMENSIONS = ['Architecture', 'Security', 'Performance', 'Correctness', 'Conformance-to-AC', 'Test-quality']
const DIMENSION_FOCUS = {
  Correctness: ' Focus on logic, edge cases and silent-wrong-answer footguns.',
  'Conformance-to-AC': ` Re-read the issue's acceptance criteria for this task and check EACH is actually met by the diff — including documentation and protocol updates — independent of the separate conformance step. Flag any unmet criterion.`,
  'Test-quality': ' Focus on whether the tests are genuine — would they fail before the fix, not hollow.',
}
const prText = String(pr || '')
const prNumberMatch = prText.match(/(?:#|pull\/)?(\d+)/)
if (!prNumberMatch) {
  throw new Error('reviewers MUST NOT be dispatched before a PR exists; commit/push/PR step must return a PR number')
}
const prNumber = prNumberMatch[1]
const reviews = await parallel(DIMENSIONS.map(dim => () => agent(
  `You are an independent ${dim} reviewer for this Quecto PR. PR/context: #${prNumber}\n\n` +
  `PRECONDITION: reviewers MUST NOT be dispatched before a PR exists; stop if this prompt does not include a PR number. ` +
  `Use the PR number only. Explicitly forbid passing a raw diff: do NOT accept a pasted raw diff in the prompt; fetch it yourself. ` +
  `Run your OWN independent review on the single dimension '${dim}'.${DIMENSION_FOCUS[dim] || ''} Read the diff with ` +
  `gh pr diff <PR>. Post findings as inline comments via the addPullRequestReview GraphQL mutation with event COMMENT. Be skeptical — report ONLY real issues. ` +
  `You are launched read-only (the write and edit tools are disabled — read_only); you keep bash/read/grep/find/agent_cmd to fetch the diff and post inline comments, but cannot create or modify repo files. ` +
  `Post findings as INLINE review comments on the PR via the GitHub GraphQL API (gh api graphql): ` +
  `fetch the PR node id and head SHA with gh pr view <PR> --json id,headRefOid, then submit one review ` +
  `carrying inline comments via the addPullRequestReview mutation (event COMMENT, comments array of ` +
  `path/line/body anchored to the head commit), or addPullRequestReviewThread per finding. If a line ` +
  `anchor is rejected (line outside diff, or PR already merged), fall back to a review comment that still ` +
  `cites file:line. Each finding: file:line, severity, the problem, a concrete fix. CRITICAL: SUBMIT the ` +
  `review (event COMMENT / submitPullRequestReview) — never leave it PENDING (a draft is invisible to ` +
  `others); verify the review state is not PENDING (submittedAt != null) before returning. Return a summary of ` +
  `posted findings.`,
  { label: `review:${dim}`, phase: 'PR Review' }
)))

// ── Fix reviews, push fixes, resolve threads ───────────────────────────────
phase('Fix Reviews')
const fixResolve = await agent(
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

// ── Conformance: systematic PR-vs-issue acceptance-criteria gate ────────────
phase('Conformance')
let conformance = await agent(
  `Task: ${TASK}. PR: ${pr}\n\n` +
  `SYSTEMATIC ACCEPTANCE REVIEW — a hard gate before merge. Read the ORIGINAL issue's acceptance ` +
  `criteria ('gh issue view <N> --json title,body,comments') and the PR diff (gh pr diff <PR>), and ` +
  `inspect the actual branch code. For EVERY acceptance criterion, decide met / partial / unmet and cite ` +
  `concrete file:line evidence — a criterion counts as met ONLY with evidence in the code, never on the ` +
  `strength of the PR description's claims. Be skeptical. Do NOT modify code. Output a per-criterion table, ` +
  `then a final line that is EXACTLY "CONFORMANCE: PASS" if every criterion is fully met, otherwise ` +
  `"CONFORMANCE: FAIL" followed by the specific unmet/partial criteria.`,
  { label: 'conformance', phase: 'Conformance' }
)
// On FAIL, run one targeted fix round against the unmet criteria, then re-verify.
if (/CONFORMANCE:\s*FAIL/i.test(conformance)) {
  log('Conformance FAIL — fixing unmet acceptance criteria, then re-verifying before merge.')
  await agent(
    `Task: ${TASK}. PR: ${pr}\n\n` +
    `The systematic acceptance review FAILED:\n${conformance}\n\n` +
    `Fix ONLY the unmet/partial acceptance criteria in the same branch; keep changes minimal; push (the ` +
    `full pre-push gate must pass). Do NOT merge.`,
    { label: 'conformance-fix', phase: 'Conformance' }
  )
  conformance = await agent(
    `Task: ${TASK}. PR: ${pr}\n\n` +
    `Re-run the systematic acceptance review after the fix round, same rules: verify each issue criterion ` +
    `against the branch code with file:line evidence; end with EXACTLY "CONFORMANCE: PASS" or ` +
    `"CONFORMANCE: FAIL" + the remaining gaps. Do NOT modify code.`,
    { label: 'conformance-recheck', phase: 'Conformance' }
  )
}

// ── Report gate: refuse a clean hand-off if any upstream phase errored or conformance failed ──
const reviewsCompleted = reviews.filter(Boolean).length
const conformancePass = !!conformance && /CONFORMANCE:\s*PASS/i.test(conformance)
if (reviewsCompleted < DIMENSIONS.length || fixResolve === null || !conformancePass) {
  const reason = [
    reviewsCompleted < DIMENSIONS.length
      ? `only ${reviewsCompleted}/${DIMENSIONS.length} reviewers completed (a reviewer phase errored)`
      : null,
    fixResolve === null ? 'the fix/resolve phase errored' : null,
    !conformancePass ? 'conformance did not return CONFORMANCE: PASS' : null,
  ].filter(Boolean).join('; ')
  log(`REPORT BLOCKED — ${reason}. Leaving PR ${pr} open for manual attention; NOT merging.`)
  return { pr, ready: false, blocked: reason, conformance }
}

// ── Report PR readiness, do not merge ─────────────────────────────────────
phase('Report')
const result = await agent(
  `Task: ${TASK}. PR: ${pr}\n\n` +
  `Acceptance-conformance verdict:\n${conformance}\n\n` +
  `STEP — Confirm the latest push's pre-push gate passed IN FULL on the latest pushed commit ` +
  `(coverage threshold, machete, deny, and the mocked @mock-llm e2e lane) WITHOUT --no-verify, ` +
  `and the required CI checks "Unit Tests" and "Mock LLM E2E Tests" are green (` +
  `gh pr checks <PR>). Also confirm reviewers posted inline findings and all threads are resolved. ` +
  `Do NOT run gh pr merge or git merge and do NOT set auto-merge. Report the PR number and summary, then stop.`,
  { label: 'report-pr', phase: 'Report' }
)

log('Feature workflow complete — PR reported, not merged.')
return { pr, result }
