# #1712 / #1720 — explicit controls reach suspended and busy members

## Mechanisms
- #1712: a parent's `agent_cmd prompt` is fast-acked and forwarded as a
  `follow_up`; on an idle member suspended by a provider failure the
  follow-up was queued and the drain returned at once while suspended, so
  the instruction never executed (receipt stuck at `queued`). Only a steer
  (which takes the genuine prompt path and re-arms) got through.
- #1720: every client connection's guard pushed a `Disconnected` sentinel
  into the bounded (256) command channel on drop. A dispatcher busy in a long
  turn (e.g. waiting for inference admission) consumes nothing, so each
  parent/TUI poll connection left one sentinel behind until the channel was
  full and the reader rejected `steer`/`follow_up` with "control queue is
  full or closed".

## Change
- `drain_and_run_pending`: while suspended, explicit instructions (user
  prompts, queued controls) are taken out of the pending queue and offered
  to the run; the member re-arms only when the run admits one (a paused run
  or a failed status probe keeps both the suspension and the instruction),
  then runs it and drains the automatic notes it had put back. A pending
  steer still outranks the drain. A failed explicit turn re-suspends as
  before. A swarm wake nudge parked behind a steer is queued as
  `PendingMessage::Automatic`: it runs like a prompt but never counts as an
  instruction.
- Disconnect sentinels travel on their own unbounded channel
  (`ClientGuard::disconnect_tx`), drained by the dispatch loop ahead of
  commands, so they never consume command-channel capacity. The receipts
  for a genuinely full channel are unchanged (explicit failure).
- Docs: `docs/swarm.md` and the agent-facing swarm guide say any explicit
  instruction re-arms a suspended member and that polls do not consume
  command capacity.

## Proof
- `an_explicit_follow_up_re_arms_a_provider_suspended_idle_member`: an
  automatic note alone stays suspended and is kept; a pending steer blocks
  the drain; the explicit follow-up then runs, re-arms, drains the kept note
  and its receipt completes.
- `an_idle_suspended_member_executes_a_forwarded_follow_up`: the fast-ack
  path (`handle_follow_up`) executes on an idle suspended member and its
  receipt completes.
- `only_an_admitted_explicit_instruction_re_arms`: paused run and failed
  probe keep the suspension and the queued follow-up (receipt stays
  queued, no provider request); a wake nudge parked behind a steer drains
  after the instruction and never re-arms on its own.
- `disconnect_sentinels_never_consume_command_capacity`: 300 dropped client
  guards against a 1-slot command channel leave a steer's `try_reserve`
  succeeding and every sentinel still reaches the dispatcher ahead of the
  next command.
- Existing suites: fast-ack conversion, queue-full rejection, steer priority,
  paused-run retention, provider-failure suspension (#1721).
