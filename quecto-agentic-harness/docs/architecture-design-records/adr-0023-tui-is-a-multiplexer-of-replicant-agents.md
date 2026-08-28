# ADR-0023 — The TUI Is a Multiplexer of Replicant Agents

**Status:** Rejected — superseded by ADR-0025.

**Implementation status:** Phase 0 (shared-state hardening, #1460) landed;
remaining phases tracked by epic #1467.

## Context

quecto-tui today drives exactly one `quecto agent --mode uds` process over one
socket. The multi-session epic (#1467) adds tabs: several concurrent sessions
visible and switchable inside one TUI.

Two topologies were considered for "several sessions at once":

1. **Multi-session within one agent process** — one agent multiplexes N
   sessions internally; the TUI keeps a single connection and addresses
   sessions by key.
2. **One replicant agent per tab** — the TUI spawns (or attaches to) one agent
   process per tab, each owning exactly one session, and multiplexes
   connections.

The `spike-tui-tabs` branch prototyped option 1 and is the evidence against
it: the agent's turn loop, context management, spill store, workflow state and
UDS session vocabulary all assume one active session per process, and the
spike had to thread a session discriminator through every one of those layers
while re-deriving per-session isolation the OS already provides between
processes. Meanwhile the whole harness stack — spawning, socket protocol,
reconnection, persistence — already works for one-agent-one-session.

Shared state is the price of the process-per-tab topology: N agents share one
XDG runtime dir of sockets, one `credentials.json`, and one session store.
Latent single-process assumptions there become routine failures at N > 1.

## Decision

**The TUI is a multiplexer of replicant agents: one replicant agent process
per tab, one session per agent.** Multi-session-within-one-agent is rejected
(`spike-tui-tabs` is the recorded evidence).

**Lifecycle inversion.** Tab agents are spawned with `--persist` and never
`--no-session` (ephemeral agents share one spill path that is scrubbed on any
exit). The agent detaches from the TUI's lifetime:

- TUI exit ≠ agent death — agents keep running and the TUI reattaches on
  restart;
- closing a tab is the explicit terminate action for that tab's agent;
- a pid + socket registry sidecar records which agents exist so the TUI can
  re-discover, reattach, and reap them across its own restarts (#1461).

**Cross-process shared-state invariants** (implemented by #1460, phase 0):

- **Session single-writer ownership.** A session key has exactly one writing
  process, claimed via an exclusive OS lock (`flock(2)`) held on a stamp
  sidecar next to the session file for the owner's lifetime. A second process
  opening/resuming an owned key is refused at open time with an explicit
  error naming the key and stamped owner pid; the kernel releases the lock on
  process death, so a crash (even SIGKILL) can never strand a key and pid
  recycling can never fake a live owner.
- **Credentials locking.** Every `credentials.json` load-mutate-store cycle
  runs under a cross-process lock file (`credentials.json.lock`), so N agents
  refreshing a rotating token serialize instead of losing each other's writes.
- **Liveness-probed reaping.** Stale-socket cleanup checks each
  `quecto-agent-*.sock` against the kernel's unix-socket table before
  unlinking; a path with a live bound endpoint is never reaped regardless of
  mtime (which is fixed at bind time), and one without is dead regardless of
  freshness. Probing by connecting is deliberately avoided: a probe connect
  is indistinguishable from a client attach and would trip the
  last-client-gone shutdown of a live non-persist agent.

## Consequences

- Per-tab isolation (memory, context, crash blast radius) comes from the OS
  process boundary instead of new in-agent machinery.
- The TUI grows connection multiplexing: per-connection correlation of
  commands, events and failures (connection-tagged `CommandSendFailure` is the
  first step; connection-scoped correlation ids follow in #1463 / ADR-0016).
- Agents outliving the TUI means orphan management is a real surface: the
  registry sidecar plus liveness probing replace "TUI closed, everything
  died" as the cleanup story (#1461 / ADR-0015 liveness dimension).
- N processes cost more memory than one multiplexed agent; accepted — tabs
  are expected to number in the single digits.

## Alternatives considered

- **Multi-session within one agent process.** Rejected: the `spike-tui-tabs`
  branch showed it re-implements process isolation inside the agent and
  touches every session-assuming subsystem at once.
- **Tabs as plain terminal multiplexing (tmux panes of independent TUIs).**
  Rejected: no shared roster, no cross-tab status, no reattach story, and each
  TUI still assumes it owns the shared files — the phase-0 invariants would
  still be needed.
- **Keep ephemeral (`--no-session`) tab agents.** Rejected: ephemeral agents
  share one spill path scrubbed on any exit, so one tab closing destroys
  another tab's spill data; `--persist` with per-session ownership is the
  safe default.
