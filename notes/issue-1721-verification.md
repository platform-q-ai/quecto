# #1721 — resume restores members suspended by provider failures

## Mechanism
A terminal provider failure suspends a member's automatic turns; wakes were
ignored while suspended, so a `swarm_control resume` never brought such a
member back and the resumer itself received no wake (the store's fan-out
excludes the sender). Observed in the #1708 swarm (`env-XKlgLrGC3m`).

## Change
- `TurnControl` tracks the latest swarm control generation seen (receipts,
  wakes, a status probe at startup), shared by the reader and dispatch loop.
- Suspensions carry a cause and the generation they were observed under
  (`AgentSession::suspend_automatic_turns`); a provider-failure suspension is
  dated by the dispatch loop right after the failed prompt.
- `handle_wake` re-arms a provider-failure suspension when the wake's
  generation is newer than the suspension's (a pause/resume happened since);
  a wake at the same generation is only a notification; store rejections
  never re-arm this way; an explicit prompt still re-arms as before.
- The reader's `swarm_control` intercept records the receipt generation and,
  on `resume`, wakes this process itself, so the coordinator that was resumed
  by its parent recovers without a prompt or steer.
- Docs: `docs/swarm.md` recovery recipe; the swarm tool description says
  pause is not a member-failure remedy.

## Proof
Session suspension unit tests, `handle_wake` re-arm test (same generation:
notification only; newer: re-armed; store rejection: stays), `TurnControl`
generation monotonicity, and the real-process BDD scenario "A resume restores
a member suspended by a provider failure": terminal 401 → suspended → parent
pause → parent resume → the coordinator runs its next turn with no prompt.
