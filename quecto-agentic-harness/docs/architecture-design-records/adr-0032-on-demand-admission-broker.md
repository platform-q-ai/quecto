# ADR-0032: The admission broker starts on demand and stops itself when idle

**Status:** Accepted
**Date:** 2026-10-21
**Scope:** #2615; narrowly supersedes [ADR-0026](adr-0026-shared-inference-admission.md)'s "No client silently starts a replacement authority" for a root whose authority does not answer.

## Context

With an `admission` section configured, the broker had to be running before
any agent started: a terminal kept open on `quecto admission-broker run`, or
the systemd user unit installed by hand. Otherwise every root failed with
"admission authority unreachable". The broker should be invisible in normal
use, the way `gpg` starts `gpg-agent`.

A broker started for a session cannot be that session's child for its
lifetime: other sessions share it, and the TUI's teardown of its own agent
must not end it. Once it is started for you, it needs a lifetime of its own:
stay up until logout, or stop itself after a long idle period.

## Decision

**Start on demand.** A root (a TUI's agent or a headless agent) whose
authority does not answer starts one before it registers:
`NegotiateAuthority::ensure_running` probes the client socket and, when
nothing accepts, launches `quecto admission-broker run --stop-when-idle`
through the `AuthorityLauncher` port, then probes again with the link's
bounded, jittered reconnect backoff. The adapter starts it detached: in a
session and process group of its own, stdin closed, stdout and stderr
appended to `broker.log` in the admission directory, working directory `/`.
It is the launching agent's child only until that agent exits, never the
TUI's, and it outlives both. The singleton lock settles concurrent starters:
the loser exits Busy (3) and its starter waits for the winner. A child never
starts a broker: it can only bind the capability its parent registered.
Only the quecto binary is ever launched.

**Stop when idle.** A broker started on demand stops itself after **one
hour** in which it held nothing: no connection open, no root scope, and no
permit, queued wait or lease outstanding, uncertain and restart-orphaned
ones included. Holding anything restarts the hour. A state it cannot read
counts as holding. The decision is made inside the broker's one serialized
turn, which ends serving in that same turn, so nothing arriving after it is
granted. A root that reaches a broker in the instant it stops makes sure
once more and starts a fresh one.

Why stop rather than stay up until logout:

- **No stale policy.** Policy is restart-only. A broker that ran for days
  keeps serving the policy it started with after the config changed; one
  that stops when idle picks up the current policy at its next start.
- **The next session starts it again.** Stopping costs one start (well under
  a second) the next time a root needs it; staying up buys nothing a session
  cannot get back on demand.
- **Nothing held is ever dropped.** A broker that holds a permit or a lease
  keeps it until the work completes, the lease clears at its bound
  (ADR-0031) or an operator resets it; the idle stop never shortens that.

**Always-on stays.** `install-service` and a foreground `run` keep serving
until they are stopped, exactly as before; `--stop-when-idle` is set only by
the on-demand launch. A service-managed broker answers the probe, so no
second broker is started; if one were, the singleton lock refuses it. With
no `admission` section nothing is probed or started.

## Consequences

- An agent with the section configured no longer needs the broker running
  first. If the broker cannot start, the agent stops before any inference
  with the reason (and the log path when the broker itself failed).
- A root whose broker dies mid-session still only reconnects (bounded
  backoff, then `unavailable`); it does not start a replacement on its own.
  A restart from the TUI is a separate command.
- **Environment and cgroup.** The broker gets an allowlisted environment
  (no provider keys or tokens linger in its `/proc/<pid>/environ` for its
  lifetime; `RUST_LOG` is fixed at `warn`). When a systemd user manager
  answers, it starts in a transient scope of its own
  (`systemd-run --user --scope`), so stopping the launching terminal's unit
  does not stop the shared broker; otherwise it relies on its own session.
- **Service race.** A session that starts a broker before the systemd unit
  does (at login) leaves the unit exited `Busy` (3), not restarted by
  `RestartPreventExitStatus=3`. `status` names this; once the on-demand broker
  stops, `systemctl --user restart quecto-admission-broker.service` restores
  the always-on service.
- **Version skew.** A session of a newer `quecto` can meet a broker started
  by an older binary. If the admission protocol version differs, joining fails
  closed (`unsupported admission protocol: …`) until that broker stops (idle,
  or SIGTERM) and the next session starts one from the new binary; the idle
  stop bounds the window.
- A broker started on demand leaves `broker.log` (owner-only, started afresh
  past 1 MiB, at start and while it runs) next to its journal.
- **Restart from a session** (`/admission restart`). The installed unit is
  restarted in place only while it is active and serving; anything else that
  serves is stopped first, so a broker started by hand with
  `quecto admission-broker run` is replaced by the unit (when installed for
  this config) or by one that stops itself when idle. Success means a new
  broker instance answers. Because it fails every live sub-agent and other
  sessions' in-flight requests once, a broker others share is restarted only
  on a confirming second request.
- **Tests.** A test that starts a real `quecto agent` with an `admission`
  section must start its broker first: an agent that finds none starts a
  production broker, which is not tethered to the test and outlives it until
  its idle stop. The test helpers say so beside `dies_with_the_test`.
- Rejected: stay up until logout (keeps a stale policy alive indefinitely);
  a configurable idle period (no measured need, more surface); the TUI owning
  the broker (it would end with the TUI that started it while other sessions
  still use it).
