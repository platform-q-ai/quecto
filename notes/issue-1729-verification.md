# #1729 — every swarm end is a resumable pause; only the supervisor lifts it

## Mechanism
`stop`, `complete`, token exhaustion and the wall-clock deadline used to be
terminal: settlement aborted and killed every non-coordinator member, closed
Python execution and the supervisor thread exited. The master outside the
swarm had no way to continue the same run (observed in the #1708 swarm and in
env-NLocPCQ8C8, whose coordinator held the run open for seven hours rather
than complete it).

## Change
- Store: `run` gains `outcome` and `outcome_reason` (migrated in place).
  `stop` (blocked/failed/budget-exhausted), `complete` (succeeded), a spent
  token budget and a passed deadline all move the run to `paused` holding
  that outcome (`stop` + `paused` events, so the control generation bumps).
  `cancelled` stays immediately terminal.
- Resume, close and extend are supervisor-only: the Python `resume` refuses
  members; the harness-only `_resume_external`, `_close`, `_extend_deadline`
  are reached through `RunControlAction::{Resume, Close, ExtendDeadline}`
  (the parent's `swarm_control` command, also `agent_cmd`). Resume refuses
  with a named remedy when it would pause again at once (deadline still
  passed, token budget still spent). Close makes the held outcome terminal
  and settles as before.
- Domain: `Snapshot.outcome`, `Snapshot::ended`, `Snapshot::admits_inference`
  (an ended run keeps its coordinator reporting); `RunStatus::proposable`.
  Application: `observed_outcome` turns a passed deadline into `Paused`;
  `settle` on an ended run cancels the coordinator's finished interpreter
  and suspends every other member; the supervisor loop keeps running across
  pause/resume and settles only on close or cancel.
- Interface: `swarm_control` `close` and `extend` (`deadline_seconds`),
  receipts and status events carry `outcome`/`reason`; pending explicit
  instructions are admitted for an ended run so the coordinator can report.
- Docs: `docs/swarm.md`, the agent-facing swarm and subagent guides and the
  swarm tool description.

## Proof
Python contract (`tests/swarm_helpers_test.py`, `tests/swarm_policy_test.py`):
stop → paused holding the outcome, members live, claims kept, member resume
refused, supervisor resume; completion held until close; deadline expiry
paused, resume refused until extended; token budget pause resumed only after a
raised budget; cancellation terminal even while ended. Rust: domain/application
unit tests (ended settle keeps the coordinator; deadline is a pause), control
parse and reader-dispatch tests for close/extend, contracts, and the swarm BDD
scenarios "A coordinator stop ends the run as a pause only the supervisor
resumes", "Completion holds success until the supervisor closes it" and
"Deadline expiry pauses the run until the supervisor grants more time".
