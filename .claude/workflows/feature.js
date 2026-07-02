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
    { title: 'PR Review', detail: 'Narrow parallel finders → adversarial verification → one posted review' },
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
  `before any implementation. RED evidence is required per new Then step and per new test assertion, not per ` +
  `test target: every new assertion must individually be shown to fail before implementation — a tautology ` +
  `cannot produce that evidence, so it is rejected by construction. Report the failing test names and the ` +
  `acceptance criteria.`,
  { label: 'red', phase: 'RED' }
)

// ── BDD Review: three narrow parallel finders (#1004) ──────────────────────
phase('BDD Review')
const BDD_ANGLES = {
  'Gherkin discipline':
    'BDD quality is foundational, so be STRICT and uncompromising: flag EVERY genuine best-practice ' +
    'deviation, not just egregious ones. Explicitly check the checklist: declarative/behaviour-focused ' +
    'scenarios with NO implementation detail in steps; strict Given-When-Then discipline (Given=context, ' +
    'When=single triggering action, Then=observable outcome); one behaviour per scenario; ' +
    'ubiquitous/domain language; no conjunctive steps hiding multiple actions; reusable, well-abstracted steps.',
  Falsifiability:
    'For EACH assertion, name the implementation change that would make it fail. Flag self-asserted ' +
    '(test-constructed) state, constant comparisons, type-level facts, and any assertion that would still ' +
    'pass with the implementation reverted.',
  Coverage:
    'Map every acceptance criterion to a scenario and flag gaps. Boundary pinning: both sides of every ' +
    'numeric/size limit must be tested.',
}
const bddFindings = await parallel(Object.entries(BDD_ANGLES).map(([bddAngle, bddFocus]) => () => agent(
  `You are a narrow, STRICT BDD review finder for the Quecto repo. Task under review: ${TASK}\n\n` +
  `Review ONLY the single angle '${bddAngle}'. ${bddFocus}\n` +
  `Read the changed BDD feature files, step tests, and unit tests (use git diff to find them). Stay ` +
  `skeptical — report ONLY real issues, never invalid or hallucinated findings. There is no verify wave — ` +
  `the scope is small. Review-only role: do NOT create or modify any repo files — no writes, no edits, no ` +
  `mutating commands (this is an instruction, not a sandbox guarantee). ` +
  `Each finding must quote the offending line and give a concrete fix, plus file:line and severity. ` +
  `Return the findings, or exactly 'NO FINDINGS' if the angle is clean.`,
  { label: `bdd-find:${bddAngle}`, phase: 'BDD Review' }
)))
const bddReview = bddFindings.filter(Boolean).join('\n\n---\n\n')
log('BDD finders complete — address EVERY valid finding regardless of severity before GREEN.')

// ── GREEN ──────────────────────────────────────────────────────────────────
phase('GREEN')
await agent(
  `Task: ${TASK}\n\n` +
  `First, address EVERY valid BDD/test-quality concern from this review regardless of severity — fix it, ` +
  `or explicitly decline it with a documented rationale (reviewers can be wrong) — before GREEN:\n${bddReview}\n\n` +
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

// ── PR Review: find → verify → single-post waves (#1004) ───────────────────
phase('PR Review')
// Wave 1 — narrow parallel finders, ONE mechanical angle each, no GitHub
// writes. Conformance to the issue acceptance criteria is NOT a finder angle —
// it needs whole-issue context and lives in the standalone Conformance phase.
const ANGLES = {
  'Hunk scan':
    'Line-by-line hunk scan seeded with diff-specific failure classes: inverted/wrong conditions, ' +
    'off-by-one, null deref, missing await, falsy-zero checks, swallowed errors, wrong-variable copy-paste.',
  'Removed-behavior audit':
    'For every deleted/replaced line, name the invariant it enforced and where the new code re-establishes ' +
    'it; a lost guard, dropped error path, or narrowed validation is a finding.',
  'Cross-file tracer':
    'Trace callers/callees/consumers of every changed symbol; flag call sites broken by new preconditions, ' +
    'changed return shapes, new errors, or ordering dependencies.',
  Security:
    'Injection, path traversal, secret leakage, unsafe deserialization, privilege or permission gaps ' +
    'introduced by the diff.',
  Performance:
    'Performance/efficiency: redundant computation or repeated I/O, sequential independent work, blocking ' +
    'work on hot paths, unbounded growth.',
  'Reuse + altitude':
    'Does new code re-implement an existing helper (name it)? Same-defect-class grep: does the defect ' +
    'being fixed exist elsewhere in the codebase? Bandaid vs. mechanism — is the fix at the right depth?',
  'Test falsifiability':
    'For each new/changed test, name the implementation change that would make it fail; reject assertions ' +
    'on test-constructed state, constants, or type-guaranteed facts, and anything that would pass with the ' +
    'implementation reverted.',
}
const prText = String(pr || '')
const prNumberMatch = prText.match(/(?:#|pull\/)?(\d+)/)
if (!prNumberMatch) {
  throw new Error('finders MUST NOT be dispatched before a PR exists; commit/push/PR step must return a PR number')
}
const prNumber = prNumberMatch[1]
const finderReports = await parallel(Object.entries(ANGLES).map(([angle, focus]) => () => agent(
  `You are a narrow ${angle} finder for this Quecto PR. PR/context: #${prNumber}\n\n` +
  `PRECONDITION: finders MUST NOT be dispatched before a PR exists; stop if this prompt does not include a PR number. ` +
  `Use the PR number only. Explicitly forbid passing a raw diff: do NOT accept a pasted raw diff in the prompt; ` +
  `fetch it yourself with gh pr diff <PR>. Review ONLY the single mechanical angle '${angle}'. ${focus}\n` +
  `Make NO GitHub writes — never post to GitHub; return structured findings only. Review-only role: use ` +
  `bash/read/grep/find to fetch the diff and inspect the code, but do NOT create or modify any repo files — ` +
  `no writes, no edits, no mutating commands (this is an instruction, not a sandbox guarantee). Be skeptical — ` +
  `report ONLY real issues. Each finding must be structured as file:line, a one-line summary, and a concrete ` +
  `failure scenario (required — a finding without one is dropped). Return the findings, or exactly ` +
  `'NO FINDINGS' if the angle is clean.`,
  { label: `find:${angle}`, phase: 'PR Review' }
)))

// Wave 2 — adversarial verification: dedupe, then one verifier per finding.
// If Wave 1 returns no findings, skip Wave 2.
const rawFindings = finderReports.filter(r => r && !/^\s*NO FINDINGS\s*$/i.test(String(r)))
let surviving = []
if (rawFindings.length === 0) {
  log('Wave 1 returned no findings — skip Wave 2 and post a clean review.')
} else {
  const dedupedJson = await agent(
    `Dedupe these Wave 1 PR-review findings for Quecto PR #${prNumber}. Merge findings that point at the ` +
    `same line/mechanism, keep the most concrete failure scenario, and record every finder angle that ` +
    `converged on the finding. Drop any finding lacking a concrete failure scenario. Findings:\n\n` +
    `${rawFindings.join('\n\n---\n\n')}\n\n` +
    `Return ONLY a JSON array, no prose and no code fences: ` +
    `[{"file":"path","line":123,"summary":"one line","failure_scenario":"inputs/state -> wrong outcome","angles":["Hunk scan"]}]`,
    { label: 'dedupe', phase: 'PR Review' }
  )
  const jsonText = String(dedupedJson || '')
  const start = jsonText.indexOf('[')
  const end = jsonText.lastIndexOf(']')
  if (start === -1 || end <= start) {
    throw new Error('Wave 2 dedupe did not return a JSON findings array')
  }
  const deduped = JSON.parse(jsonText.slice(start, end + 1))
  const verdicts = await parallel(deduped.map((finding, i) => () => agent(
    `You are an adversarial verifier for ONE PR-review finding on Quecto PR #${prNumber}. Your job is to ` +
    `REFUTE it. Fetch the diff yourself with gh pr diff <PR> and read the surrounding code. Read-only: no ` +
    `repo writes, no GitHub posts. The finding:\n${JSON.stringify(finding)}\n\n` +
    `Return a verdict that STARTS with exactly one of CONFIRMED, PLAUSIBLE or REFUTED, then quote the proving/` +
    `disproving line. CONFIRMED = you can name the inputs/state that trigger it and the wrong outcome. ` +
    `PLAUSIBLE = the mechanism is real but the trigger is uncertain; state what would confirm it. ` +
    `REFUTED = factually wrong or guarded elsewhere — quote the line that proves it.`,
    { label: `verify:${i}`, phase: 'PR Review' }
  )))
  surviving = deduped
    .map((finding, i) => ({ ...finding, verdict: String(verdicts[i] || '') }))
    .filter(f => f.verdict && !/^\s*REFUTED/i.test(f.verdict))
}

// Wave 3 — the master posts exactly one submitted review.
const reviewPost = await agent(
  `You post the single review for Quecto PR #${prNumber}. Surviving verified findings (JSON):\n` +
  `${JSON.stringify(surviving, null, 2)}\n\n` +
  `Post exactly one submitted review via the GitHub GraphQL API (gh api graphql): fetch the PR node id and ` +
  `head SHA with gh pr view <PR> --json id,headRefOid, then submit ONE review with the addPullRequestReview ` +
  `mutation (event COMMENT, comments array of path/line/body anchored to the head commit) carrying EVERY ` +
  `surviving finding as an INLINE review comment on the PR. If a line anchor is rejected (line outside ` +
  `diff), include that finding in the review body instead, still citing file:line. Where multiple finder ` +
  `angles converged on the same line ("angles" lists more than one), treat that as a severity signal and note it ` +
  `on the finding. If there are no surviving findings, submit one review stating that no findings survived ` +
  `verification. CRITICAL: SUBMIT the review — never leave it PENDING (a draft is invisible to others); ` +
  `verify the review state is not PENDING (submittedAt != null) before returning. Return a summary of the ` +
  `posted review.`,
  { label: 'post-review', phase: 'PR Review' }
)

// ── Fix reviews, push fixes, resolve threads ───────────────────────────────
phase('Fix Reviews')
const fixResolve = await agent(
  `Task: ${TASK}. PR: ${pr}\n\n` +
  `Verified findings posted inline (JSON):\n${JSON.stringify(surviving, null, 2)}\n\n` +
  `Posted-review summary:\n${reviewPost}\n\n` +
  `STEP — Fix all valid review concerns. Triage each inline finding; confirm it is genuinely valid before ` +
  `changing anything (reviewers can be wrong). Address EVERY valid concern regardless of severity — not just ` +
  `high-priority ones: fix it in the same branch, or explicitly decline it with a documented rationale. ` +
  `Track accepted vs declined.\n` +
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
  `concrete file:line evidence — including any documentation and protocol updates the criteria require. ` +
  `A criterion counts as met ONLY with evidence in the code, never on the ` +
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
const findersCompleted = finderReports.filter(Boolean).length
const angleCount = Object.keys(ANGLES).length
const conformancePass = !!conformance && /CONFORMANCE:\s*PASS/i.test(conformance)
if (findersCompleted < angleCount || !reviewPost || fixResolve === null || !conformancePass) {
  const reason = [
    findersCompleted < angleCount
      ? `only ${findersCompleted}/${angleCount} finders completed (a Wave 1 finder errored)`
      : null,
    !reviewPost ? 'the Wave 3 review post errored' : null,
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
