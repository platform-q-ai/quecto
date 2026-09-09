# Shared inference admission (#1679)

Quecto can bound how many provider requests all of one user's cooperating
sessions and sub-agents on one host have in flight at once, with request
pacing, fair queueing and a shared cooldown when a provider throttles. It
limits **outbound provider attempts**, never the number of agents: spawning,
tool execution, idle agents and parents waiting on children hold no permit.

Admission is **disabled by default**. Nothing changes until an `admission`
section is configured and the authority process is running.

## Configuration

```json
{
  "admission": {
    "directory": "/home/me/.quecto/admission",
    "groups": {
      "openai-main": {
        "capacity": 4,
        "reserve": 1,
        "min_interval_ms": 250,
        "queue_capacity": 64,
        "queue_timeout_ms": 120000,
        "attempt_timeout_ms": 900000,
        "fallback_base_ms": 2000,
        "max_cooldown_ms": 600000
      }
    },
    "aliases": { "acct-openai": "openai-main" },
    "bindings": { "openai": "acct-openai" }
  }
}
```

- `groups`: one entry per provider/account quota you share. `capacity` is the
  bound `C`, `reserve` the interactive-only slots `R` (`0 <= R < C`),
  `min_interval_ms` the pacing between request starts, `queue_capacity` and
  `queue_timeout_ms` bound waiting, `attempt_timeout_ms` bounds one attempt,
  `fallback_base_ms`/`max_cooldown_ms` govern throttle cooldowns.
- `aliases`: opaque, non-secret account/endpoint names mapped to a group.
  Credentials, endpoint URLs and model names are never used as identity.
- `bindings`: router provider slot (`openai`, `anthropic`, or an
  `openai_compatible` endpoint `prefix`) to alias. Every slot you want bounded
  must be bound explicitly; unknown aliases fail configuration.
- `directory` (optional, default `<base_dir>/admission`): a private, owner-only
  directory holding the authority's lock, journal and sockets. It must be
  absolute and must not be the base directory or one of its ancestors; it is
  refused if group/other bits are set.

No numeric value above is a vendor-safe default; measure and set your own.
Policy and bindings are restart-only: a reload with a changed section is
rejected and the live runtime keeps its current values.

## Running the authority

```sh
quecto admission-broker run      # holds the singleton lock until SIGTERM/SIGINT
quecto admission-broker status   # JSON: epoch, journal health, per-group counts
quecto admission-broker reset    # new accounting epoch (see recovery below)
```

`run` prints `admission authority ready: <socket>` on stderr. A second `run`
on the same directory exits with status 3. Stopping the authority does not
kill sessions, but capabilities live only in the authority process: after a
restart every running session's next attempt fails explicitly and the session
must be restarted to register again. A ledger that is missing after prior
operation is refused; `run --accept-missing-ledger` explicitly starts an
empty one. Roots are minted with the owner token at `<directory>/root.token`
(0600, outside `client/`), so a process that only sees the client directory
can bind a pre-registered child capability but never promote itself to a root.

Every `quecto agent` started with the section configured registers itself as
a root at the authority before composing its provider. If the authority is
unreachable the agent exits before any inference. Sub-agents receive a
pre-registered capability through a private sidecar file and bind it before
announcing socket readiness; a forged or revoked capability exits non-zero.

## Containers

Script-managed container children need the authority's `client/` directory
mounted at the same path. The parent passes `QUECTO_ADMISSION_DIR` to the
`create`/`exec` scripts; the bundled Docker/Podman adapter mounts it and
reports `"admission_capability": "shared-directory-v1"`. An admission-enabled
parent refuses to launch a container whose script does not report that
capability. Only admission operations are exposed through the mount; the
journal, the admin socket and the owner token stay outside it. The bundled
adapter also identity-mounts `$HOME/.quecto`; when the authority directory
lives under it (the default `<base_dir>/admission`), the adapter masks the
authority root with an empty tmpfs and re-exposes only `client/`.

## Failure safety and recovery

- Every grant is written to the journal (file and directory fsync) before it is
  visible; if the journal cannot be written, no grant is issued.
- A session that exits with nothing outstanding releases its scope on
  disconnect; only unverified work keeps a scope registered.
- A client that disconnects or is killed while an attempt is outstanding leaves
  **uncertain** occupancy: the group stops granting (quarantine) because the
  remote work may still be running. The same session reconnecting and
  completing the attempt clears it.
- An authority restart keeps the epoch and turns outstanding work into
  orphaned uncertain occupancy; it never restarts with an empty ledger.
- `quecto admission-broker reset` starts a new epoch, revokes every
  capability (sessions must restart) and explicitly gives up any claim that
  old remote work is bounded. Cooldown and pacing deadlines survive the reset.

## Observing admission

Admission is orthogonal to a session's lifecycle: a queued attempt is neither
idle nor stalled. Each process records its own attempts' transitions (waiting,
admitted, completed, refused, cancelled, abandoned) and the cooldown it last
learned per quota group, as a bounded and fresh view (at most 64 live attempts
are sampled, the rest are counted as `hidden`; refusal reasons are bounded to
200 bytes).

Over the socket the view rides on `get_state` as the `admission` object next
to the execution phase, and the progress verdict becomes `waiting` (with the
count, group, longest wait and cause) for as long as any attempt is queued.
A changed admission revision advances the `get_state` generation once per
observation, and every transition is pushed in order as an
`admission_state_changed` event; elapsed waits and cooldown remainders are
re-derived on each full read rather than being transitions. See
[uds-protocol.md](uds-protocol.md).
`abort` while an attempt waits cancels the wait at the authority and counts
it under `counters.cancelled`.

## Activation, rollback and quarantine runbook

Activation (per host, per user):

1. Add the `admission` section (groups, aliases, bindings) to the config;
   every provider slot that must be bounded needs a binding. Measure your own
   values; none of the numbers in this document are vendor-safe defaults.
2. Start the authority: `quecto admission-broker run` (a service or a
   terminal that outlives every session). It refuses to start twice on the
   same directory.
3. Verify: `quecto admission-broker status` reports `journal: healthy`, epoch
   `1` and zero active/queued/uncertain counts per group.
4. Start (or restart) every agent process. Roots register before composing a
   provider, so a session that cannot reach the authority exits with an error
   before any inference; a running session never switches policy in place.
5. Check a session: `get_state` carries `admission`, and a queued attempt
   shows `progress.state = "waiting"` (TUI: "⏳ waiting for admission").

Rollback:

1. Remove the `admission` section (or stop binding the provider slots).
2. Restart every agent process; a live reload with a changed section is
   rejected by design, so nothing changes until the restart.
3. Stop the authority (SIGTERM). Outstanding remote work is no longer
   bounded from that moment; rollback does not pretend otherwise.

Quarantine (a group has stopped granting):

1. `quecto admission-broker status` shows `uncertain > 0` for the group: a
   session disconnected or was killed while an attempt was outstanding, or the
   authority restarted with work in flight.
2. If that session is still alive and reconnects with the same capability,
   completing the attempt clears the entry; wait for it.
3. Otherwise `quecto admission-broker reset` starts a new epoch. It revokes
   every capability, so every session must restart, and it gives up the claim
   that the old remote work is bounded. Cooldown and pacing deadlines survive.
4. A group in cooldown (`get_state` `admission.groups[].cooldown`) needs no
   action: the deadline came from the provider's own advice. `unavailable`
   means the provider advised beyond the configured maximum; treat it as a
   provider incident, then reset.

## Limitations

- Same-user, same-host coordination only. Processes holding provider
  credentials can still bypass it; other hosts and other users are not covered.
- Concurrency and pacing do not guarantee provider RPM/TPM compliance; token
  accounting and adaptive concurrency are follow-on work.
- Closing a connection does not cancel remote provider computation.
- The TUI shows a session's own wait and forwarded descendant waits; queue
  position is never promised (ADR-0026).
