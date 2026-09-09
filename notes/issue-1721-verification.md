# #1721 — resume restores members suspended by provider failures

## Mechanism
A terminal provider failure suspends a member's automatic turns; wakes were
ignored while suspended, so a `swarm_control resume` never brought such a
member back and the resumer itself received no wake (the store's fan-out
excludes the sender). Observed in the #1708 swarm (`env-XKlgLrGC3m`).

## Change
- `TurnControl` tracks the latest swarm control generation seen (receipts,
  wakes, a status probe spawned at startup), shared by the reader and
  dispatch loop.
- Suspensions carry a cause and the generation they were observed under
  (`AgentSession::suspend_automatic_turns`); after a failed prompt the
  dispatch loop probes `Status` and dates a provider-failure suspension by
  the generation current *after* the failure (prompt, drained and nudged
  turns alike; a suspension already dated is never re-dated), so a
  pause/resume that happened during the turn cannot re-arm it.
- `handle_wake` probes `Status` while suspended (no wake events claimed) and
  re-arms a provider-failure suspension when the control generation is
  newer than the suspension's; a wake at the same generation is only a
  notification; store rejections never re-arm this way; a failed probe keeps
  the member suspended (contention defers the generation into the next
  reader-delivered wake without occupying the coalescing slot, so later
  wakes are never swallowed; either way an event says so); an explicit
  prompt still re-arms as before. A re-armed member
  owes one resume turn, run once the run admits inference; a prompt or a
  later suspension cancels that debt.
- A resume wakes every live member with its control generation from both
  resume paths (the control port behind `swarm_control` and the `swarm`
  tool op): the store's notification policy targets nobody for a resume.
  Members that could not be reached ride the receipt as `wake_warnings` on
  both paths. The reader's `swarm_control` intercept also wakes its own
  process (the fan-out excludes the sender), releasing the coalescing slot
  with a warning when the command channel is full or closed.
- Docs: `docs/swarm.md` recovery recipe, the agent-facing swarm guide and
  the swarm tool description say pause is not a member-failure remedy.

## Proof
Session suspension unit tests, `handle_wake` tests (same generation:
notification only; newer: re-armed; re-armed while paused: turn owed until
running; failed probe: stays suspended; store rejection: stays), post-turn
dating (prompt and drained turns; dated suspensions untouched), deferred
wake folding after a transient probe failure, resume fan-out over a real member socket (both paths, unreachable
member reported), `TurnControl` generation monotonicity, and the real-process BDD scenario "A resume restores
a member suspended by a provider failure": terminal 401 → suspended → parent
pause → parent resume → the coordinator runs its next turn with no prompt.
