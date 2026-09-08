# Swarm trial regression verification

Baseline: `d8490741dce0fdcbf9240e8d53d96cc45b2983a0` (merged PR #1684).
Working branch: `codex/swarm-live-trial-regressions`.
Diagnostic input: `/tmp/issue-1671-swarm-trial-diagnostics.md`. The original container is gone; its actual dispatch trace and database cannot be reconstructed from this report. These are controlled local reproductions, not a claim that issue #1671 made verified implementation progress.

## Observed RED, then GREEN

- Real SQLite: a consumed message still selected its recipient for a wake; task creation followed by immediate claim still woke an idle peer. `/tmp/swarm-trial-python-red.log` records both failing assertions.
- Real SQLite: identical blocker/evidence writes repeatedly generated new wake hints. `/tmp/swarm-trial-repeat-red.log` records both failures. Changed blockers still notify; unchanged updates are no-ops.
- Rust: bounded tool-only reports omitted newest progress; Bash inherited live launch metadata; forwarded steering lost request correlation; pending work ran ahead of an admitted steer. All four assertions failed before the corresponding production changes (`/tmp/swarm-trial-rust-red.log`).
- Runtime construction: a subprocess runs the existing runtime-profile test under inherited swarm launch metadata against an actual full SQLite pool. It failed with the reported admission-cap error (`/tmp/swarm-trial-runtime-red.log`). Explicit context injection fixes it; actual managed admission still rejects an extra member. Namespace metadata is emulated for this test; no container isolation claim is made.
- Dispatch: a queued follow-up cleared the admitted-steer flag and ran ahead of the operator instruction (`/tmp/swarm-trial-queued-red.log`). Only the steering handler or abort now releases that gate.
- Dispatch: the pending buffer silently dropped a full-queue instruction while reporting success (`/tmp/swarm-trial-pending-full-red.log`). It now emits a correlated rejection.

Compilation failures encountered while writing tests were corrected and are not counted as behavioral RED evidence. Eleven failing behavior assertions were observed before their fixes. Additional positive/boundary tests were added afterward: dependency unlocks, steering during an already-running pending batch, large incomplete board history retaining a latest acknowledgment, and a BDD scenario through the execution adapter.

## Architecture and contracts

Notification policy consumes plain task/message state supplied by the atomic repository port; SQL stays in infrastructure. Selection and cursor advancement use the same transaction. Explicit runtime context is supplied by the production composition root instead of being discovered inside a reusable builder. Bash strips only swarm launch metadata; ordinary environment and actual spawn admission policy remain intact.

Pending delivery is separated into a small interface module, retains deferred messages, and gives admitted steering priority. Immediate transport acceptance, pending retention (`queued`), rejection, and actual transcript evidence are distinct. Correlation is preserved through forwarded control dispatch. Neither an acceptance response nor a completed turn proves the requested task succeeded.

The large incomplete-history test already retains a latest substantive acknowledgment. It is counterevidence to a blanket claim that report ordering is broken. The report change fixes the reproduced no-final-answer progress case. Incomplete-history cursors remain conservative: omitted history is not silently acknowledged. Already queued wake hints can become stale after send-time validation; this change does not add a new durable control receipt API or a full wake-outbox protocol.

## Local validation

- 4,092 harness library tests passed, including real-socket clarification/action tests.
- 37 Python policy/SQLite tests passed.
- 46 architecture checks, 247 contract checks, one swarm agent-loop check, and five product checks passed.
- 37 swarm BDD scenarios / 160 steps passed; strict workspace/all-target Clippy and repository quality/BDD gates passed.
- Logs: `/tmp/swarm-trial-harness-final.log`, `/tmp/swarm-trial-python-final.log`, `/tmp/swarm-trial-all-regressions.log`, `/tmp/swarm-trial-bdd-final.log`, `/tmp/swarm-trial-clippy-final.log`.

No new live-provider/container trial, issue-1671 implementation result, or new CI run is claimed. The shared master checkout and unrelated work were preserved.

## CI follow-up: idle steering receipt

CI run `34258504890` exposed the same missing idle-steer acceptance response in both BDD suites: a failing provider emitted `agent_error` before any successful `steer` response. The existing `UDS steer while idle is acknowledged` scenario reproduced locally as RED (`/tmp/swarm-trial-ci-steer-red.log`), then passed all six steps after restoring the explicit acceptance response before executing the turn (`/tmp/swarm-trial-ci-steer-green.log`). Provider failure remains visible and acceptance does not claim successful execution. No fixture, test requirement, or threshold was weakened.


## Independent adversarial-review follow-up

A fresh independent source-guided review of `f63d024f` reported two P2 control-admission gaps in `/tmp/swarm-trial-adversarial-review-f63d024f.md`: malformed steering left a stale gate after parse rejection, and one handled steer cleared priority for a second already-admitted steer. The former was an introduced recovery regression; the latter exposed an incomplete repair of the preexisting boolean design. Parent verification reproduced both as failing behavior tests (`/tmp/swarm-trial-review-red.log`).

Steering classification and control normalization now validate the same typed command accepted by dispatch, including message and ID types. Outstanding steering intent is counted; each handler consumes only its own admission, while abort/reset clears all intent. Tests cover malformed raw/accepting controls without cancellation and two admitted steers running before buffered hints. The independent review followed the repository workflow's source guidance; it was not a workflow-engine execution and is not represented as one of the required engine runs.

### Focused adversarial re-review: duplicate JSON fields

The independent re-review found that accepting control forwards validated a parsed JSON value, which had already collapsed duplicate fields, while eager steering admission validated the original line. A duplicate-message steering command could therefore be accepted without cancellation or a matching pending-steer count. The ReaderDispatch regression reproduced this mismatch (RED), and control forwarding now validates the original line with the same typed parser before normalization. This is a source-guided independent review, not a built-in workflow-engine run.
