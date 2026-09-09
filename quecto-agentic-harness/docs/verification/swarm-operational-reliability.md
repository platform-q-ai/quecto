# Swarm operational reliability acceptance ledger

Source: retained issue-1671 parent audit and export (257 checksums, a consistent
SQLite backup, and four session logs). The workload checkout SHA is not the
running harness build identity; that trial's runtime revision remains unverified.
Private logs and exports are not included in this repository.

## Behavior and regression evidence

Each behavior change began with a failing regression, followed by implementation
and refactoring. Compiler/fixture failures are not counted as behavioral RED tests.

| Area | Implemented behavior and regression coverage |
| --- | --- |
| Blockers | Owned blocked-to-claimed transition preserves reservations/token; submitted evidence cannot reopen. Missing `unblock` reproduced before implementation. |
| Pause/resume | Durable pause freezes the deadline and denies inference, retries, tools and wakes. Generation-scoped cancellation protects resumed turns and Python jobs. Terminal coordinator remains available for read-only reporting. |
| Wake delivery | Atomic notification generation, receiver actionability/frontier checks, queued-generation coalescing, paused queue retention, terminal notice suppression. No automatic retry after provider failure. |
| Reporting | Compact summaries/cursor deltas, explicit paginated events, cursor-neutral latest report, nullable unpersisted ordinals, stable recovery IDs, explicit retained-record exports. Worker join uses the same generation-bearing snapshot contract. |
| Controls | Correlated queued/started/completed/failed/cancelled/rejected receipts; full-queue rejection; direct prompt cancellation/rejection; priority supervisor pause and descendant routing. Completion describes a turn, not semantic compliance. |
| Provider failures | Ordinary/OAuth retry admission checks, terminal quota classification including nested usage-limit codes, bounded reset-aware delay, automatic-turn circuit and failure-notice deduplication. |
| Accounting | Observed usage/cache/cost fields remain distinct from estimates. Cancellation-safe idempotent outbox, bounded per-request/member diagnostics, optional budget warning/pause and strict unknown handling. |
| Provenance | Process identity and optional build metadata; background hash reads the running executable inode even during atomic replacement. Runtime digest enrichment does not recount a request. |

Specific adversarial RED cases included a stale pause cancelling a newer turn,
a stale pause cancelling a resumed Python job, terminal Bash execution, cancelled
usage disappearing before durable recording, slow export blocking same-connection
controls, and executable replacement causing the wrong digest. Each received a
regression and a fix; the integrated library checkpoint passed 4,152 tests before
the final pending-queue and direct-receipt additions.

## Independent review adaptation

Two source-guided independent adversarial review cycles were performed. The
built-in workflow runner was unavailable in this editing environment; these are
not represented as workflow-engine runs.

1. Verified and fixed cancellation scope, terminal tool admission, cancellation
   accounting, and runtime observation idempotency concerns.
2. Verified and fixed Python job generation scope, export reader blocking, and
   running-executable provenance concerns.

Local validation: 4,156 library tests, 46 architecture checks, 252 contract tests,
six swarm integration tests, and 41 Python helper tests passed. Strict workspace
Clippy (all targets and test-support), formatting and static quality gates passed.
One full-library attempt hit the existing OAuth closed-port test's HTTP 404 race;
the unchanged full-suite rerun passed. Swarm BDD passed all 40 scenarios (175 steps).
Post-CI review results are recorded in the PR description.

## Operational limits

- Resume restores admission/deadline; send an explicit prompt to continue retained
  instructions. A handling receipt does not prove the model followed instructions.
- Notifications remain best effort; durable inbox and board state are authoritative.
- Request counters measure instrumented calls, not independently observed network
  transactions. Reported costs are estimates, not provider billing verification.
- Budgets default off. In-flight concurrent requests can overshoot before accounting
  arrives. Unknown usage is not zero. Ledger capacity is 10,000 observations.
- Export is explicit, at most two concurrent exports on the multi-client reader,
  and limited to 256 MiB of output per artifact. Records reflect retained wire
  messages and available spill data; cleared records and concurrent appends are
  not guaranteed. Export files persist until cleanup and require container artifact
  transport to reach the host.
- Runtime revision and digest can be unknown/pending. No workload SHA is substituted.
- This verification does not claim a successful replay of the original live trial
  or causally attribute its reported token volume to a single defect.

CI follow-up: stale slim-state fixtures were updated for the new diagnostics.
Live-child fixtures used an obsolete provider route and non-streaming response;
explicit API routing plus a valid stream restored prompt/follow-up delivery without
weakening the failure circuit. Seven targeted BDD scenarios passed. The library-only
coverage lane now executes the shared swarm contracts and adapter failure tests:
4,179 tests passed with 92.24% function coverage locally (92% required).

A further BDD scenario drives a real UDS child with a synthetic, private swarm
launch contract and scripted provider. It verifies in-flight pause, queued approval
retention, resume and completed receipt, observed-budget pause, raw evidence export,
and terminal coordinator reporting. It passes locally; it makes no claim of actual
PID-namespace isolation. The complete prior BDD run passed its scenarios but exposed
a separate 72% function-coverage gap, which this production-path scenario addresses.

The next CI run passed seven lanes but timed out waiting for a BDD completion
event. Inspection found the polling client discarded buffered read-ahead at each
poll deadline and partial frames on socket timeouts. Two deterministic framing
regressions failed against the extracted old reader, then passed with a persistent
reader and byte buffer, including a UTF-8 character split across timeout boundaries.
This repairs the test consumer without weakening production completion assertions.

CI then exposed a separate paged-history fixture startup race (`ConnectionRefused`
after the socket path appeared). The fixture now waits for its actual connection
within the same five-second budget, retrying only not-found/refused startup states;
a pathname alone is not treated as listener readiness.

The final independent review at `778eb48d` found a public-entry-point gap:
`usage` and `usage_budget` were published and implemented in the control adapter
but omitted from `SwarmTool::execute` dispatch. Two regressions invoked the public
Tool port and failed with `unknown op` for terminal usage reporting and budget
configuration. The explicit dispatch allowlist now includes both operations.

## Adversarial review follow-up (2026-09-09)

An independent review of the PR diff raised 16 findings (1 high, 6 medium,
9 low); each was fixed with a failing test first:

- members joining a paused run are supervised (`needs_supervision`)
- the loop admits a logical request once; retries and OAuth resends re-check
  (no interpreter spawn before the first attempt), and a denied resend keeps
  the refreshed provider
- the cancel slot is never held across the coordination-status subprocess
- cancelled/rejected attempts are not unknown usage, so a strict budget does
  not re-pause forever after a supervisor pause
- the `session_usage` log rides the production `record_usage` path
- a full request ledger drops the record with a warning instead of blocking
  every turn; the ledger cap and payload bound have tests
- descendant `get_state`/`get_report` forwarders are bounded (8) and the
  reorder is documented; `get_report` resolves through the ledger
- wake requests coalesce when the command channel is full, and only durable
  rejections (not SQLite contention) stop automatic turns; a paused run
  retains its wake frontier
- `runtime_identity` test moved to a sidecar; poison-tolerant locks; the
  bounded-events drain stops at EOF; the export test no longer leaks a permit
- documented: `swarm op=pause` suspends the calling turn; Retry-After hints
  above the wait budget fail immediately (all sessions); strict-budget resume
  caveat

## Second adversarial review (2026-09-09)

Verified 13 of the 16 first-round fixes closed and 3 partial; 10 new findings,
all fixed test-first: committed Python bytecode removed and ignored; a wake
arriving on a full command channel is refused when nothing would drain it
(coalesced only when a wake is already pending) and `Closed` is an error;
wake outcomes documented; streaming re-initiations, not the first attempt,
re-check admission; SQLite schema locks count as contention; a contended wake
keeps its generation for the next wake; strict-budget wording corrected; the
whole-result usage accumulator is test-only; supervision and store-failure
guards are affirmative with typed classification (`StoreFailure`,
`AccountingFailure`); poison-tolerant locks in the turn helpers; any durable
store rejection of a diagnostic record drops it with a warning so diagnostics
never block inference, while contention keeps it pending.
