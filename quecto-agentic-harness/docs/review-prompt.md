Goal: Independent effectiveness and simplicity review of PR <PR> (delivering <ISSUE><, part of epic EPIC>), bound to head <HEAD_SHA>. Re-check the head SHA before starting; if it moves, stop and ask the parent which revision to review. Judge whether the change actually works and holds up, and whether it is as small as it can be — not merely whether it matches the issue text.

Setup and rules:
- Fresh review swarm with its own container and checkout; a fixed small pool chosen up front; member_limit 25.
- Read-only. No source edits in the shared checkout, no mutation tools or mutation replay. Any probe that needs edits runs in a private `git worktree` under /tmp with a hard timeout (SIGKILL), cleaned up afterwards.
- Targeted tests only; full suites belong to CI.
- Form your own conclusions from the issue (and epic), the project's architecture reference and rules (<PROJECT_RULES, e.g. the target-architecture wiki page, AGENTS.md, CONTRIBUTIONS.md>) and the source — before reading the PR description or any earlier review reasoning.

How to run it — fan out, then try to refute:
1. Finders. The coordinator creates one board task per review angle, and each member claims one angle. Scale the fan-out to the diff: a small diff merges angles into 2–3 finders; only a large or risky diff runs the full set. Angles:
   - hunk scan: every changed hunk, line by line;
   - removed behaviour: for each deleted or changed line, name the invariant it enforced and where it is now re-established;
   - cross-file tracer: callers and callees of every changed symbol, including configuration, wiring and docs;
   - effectiveness and false positives (below);
   - simplicity and efficiency (below);
   - security (guards affirmative, no secret or member text in logs), and performance on hot paths;
   - test falsifiability: for each new or changed test, name the implementation change that would make it fail. A test with no such change can never fail.
2. Finding format. Every finding is: file:line, a one-line summary, and a CONCRETE failure scenario (inputs or state → wrong outcome, or the probe that showed it). A finding without a scenario is dropped. A finder whose angle is clean reports "NO FINDINGS" for that angle, and says what it probed.
3. Verification. The coordinator dedupes findings that point at the same line or mechanism, then assigns each surviving finding to a DIFFERENT member from the one who found it. That verifier's job is to REFUTE the finding. The verdict starts with CONFIRMED (it names the triggering inputs and the wrong outcome), PLAUSIBLE (the mechanism is real but the trigger is uncertain; state what would confirm it), or REFUTED (quote the line or test that disproves or guards it). Only an explicit REFUTED drops a finding. An empty or errored verdict keeps it as PLAUSIBLE. "Not a behaviour defect" or "the issue did not ask for it" is not a refutation; route spec-level weaknesses to spec findings instead.

Effectiveness — prove each answer with concrete probes (tests, temp fixtures, targeted runs), not by reading alone:
- Acceptance: list every criterion of the issue and show the behaviour or test that demonstrates it. Flag any criterion that is only claimed.
- Guarantees: for every invariant, check, guard, limit or policy the change introduces, try to break it — the inputs and states it should reject, boundaries, and failure paths (missing, unreadable or malformed inputs, concurrency, timeouts, restarts). It must fail loudly where it should and never pass silently. Check that a failure is actually visible where it matters (for example, output of passing tests is hidden in CI).
- False positives: probe every guard in legitimate environments too, not only hostile inputs — checkouts under symlinked parent folders, macOS temp paths, other platforms and containers, configuration off and on, and files that already exist in places the next changes will touch. A guard that fails correct work is a defect.
- The spec itself: if the issue's own wording permits a weakness (a check that can drift, a guarantee that only holds by convention, a requirement that makes the next change hard), report it as a finding against the spec — do not pass it because the code matches the text.
- Tests: find any test or check that can never fail, and name concrete inputs the tests would miss.
- The next change: if later work breaks it, does the failure message tell that author exactly what to fix?
- Wiring and telemetry: is it actually used in production (not only in tests), and does it record what the improvement loops need?

Simplicity and efficiency:
- What is the smallest implementation that fully meets the issue and the project rules (layering, allowlist guards, defensive handling, assertions)? Compare with the diff's size.
- Name code that validates the change's own hard-coded data, anything written in two places (in code or copied into docs), single-use abstractions, speculative generality, and scope beyond the issue. Estimate the lines each simplification removes.
- Runtime cost on hot paths: needless clones, allocations, I/O, lock holding, polling, extra round trips.
- Future cost: is the next related change a one-place edit? Keep what is genuinely needed, and say why.

Repository hygiene: red-first history intact where required, version bumped in the PR's own final commit, formatting and lint clean, no process artifacts in the diff.

Output:
- The coordinator posts EXACTLY ONE submitted PR review (event COMMENT) with every surviving CONFIRMED or PLAUSIBLE finding as an inline comment on the head commit, ranked High / Medium / Low, each with its verdict, scenario, impact and expected correction.
  - Anchor each comment to the nearest CHANGED line of the diff, and always cite the real file:line in the comment body, wherever the anchor landed.
  - API gotcha: GraphQL `addPullRequestReview` takes legacy diff `position` (the offset within the hunk), not line numbers. For line anchoring use `addPullRequestReviewThread`. When an anchor is rejected either way, put that finding in the review body, still citing file:line.
  - Verify the review's `submittedAt` is set, so it is never left pending and invisible. If nothing survived, submit one review saying so.
- The review body also carries:
  - the verdict: mergeable / mergeable after fixes / needs rework;
  - the scorecard (effectiveness, test strength, simplicity, runtime efficiency, maintainability), each with one line of evidence;
  - the spec findings;
  - what was REFUTED, and why;
  - the estimated size of the minimal version;
  - the limits of what was not probed.
- Report the same summary to the parent. Before closing the swarm, confirm the shared checkout is untouched (`git status` clean) and no probe worktrees remain.
- Never fix, commit, push or merge.

Criteria:
- probed-not-read (review): every acceptance criterion and introduced guarantee has concrete probe or test evidence, including false-positive probes in legitimate environments
- adversarially-verified (review): every finding has a scenario and a verdict from a member other than its finder; only explicit REFUTED findings were dropped
- spec-challenged (review): weaknesses the issue text permits are reported as spec findings rather than passed
- simplicity-assessed (review): a minimal-size estimate with named simplifications is given
- one-submitted-review (review): exactly one submitted review exists on the head commit, anchored or cited as required
