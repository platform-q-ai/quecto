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
