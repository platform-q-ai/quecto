# Admission broker

Shared inference admission bounds outbound provider attempts across cooperating
sessions and subagents for one user on one host. It is **disabled by default**:
omitting `admission` leaves inference unbounded by this broker. When configured,
an unreachable authority is an error, never an unbounded fallback.

## Configure and start

Example config fragment (illustrative values, not vendor-safe defaults):

```json
{
  "admission": {
    "groups": {
      "shared": {
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
    "aliases": { "account": "shared" },
    "bindings": { "openai-oauth": "account", "openai-api": "account" }
  }
}
```

All eight group fields are required; none has a numeric default. `capacity`
bounds in-flight attempts; `reserve` holds interactive-only capacity, so
background attempts can use at most `capacity - reserve`. Require capacity > 0,
0 <= reserve < capacity, positive queue capacity and all intervals/deadlines,
and fallback_base_ms <= max_cooldown_ms. Pacing (`min_interval_ms`) separates
request starts even when capacity is free. `queue_timeout_ms` bounds admission
waiting; `attempt_timeout_ms` bounds an admitted provider attempt.
`fallback_base_ms` supplies throttle backoff without provider advice;
`max_cooldown_ms` bounds acceptable cooldown advice.

Optional section fields and actual defaults:

| Field | Default | Purpose |
| --- | --- | --- |
| `directory` | `<base_dir>/admission` (normally `~/.quecto/admission`) | Private authority storage and sockets |
| `max_scopes` | `1024` | Lifetime issued-scope bound, including retired replay fences |
| `terminal_capacity` | `4096` | Bounded retained terminal request records |

Both capacity fields above must be positive. An explicit directory must be
absolute, owner-only, and below or separate from the base directory (never the
base directory or its ancestor). `groups`, `aliases`, and `bindings` are required;
every group needs an alias. Unknown fields and invalid configurations are rejected.

Bindings map exact router provider slots to opaque aliases, which map to quota
groups. Bind **every constructed provider**, not just the selected model.
Built-ins are `openai-api`, `openai-oauth`, `anthropic-api`, and
`anthropic-oauth`; bare `openai`/`anthropic` do not bind them. Custom registry
providers use their provider key; `openai_compatible` uses its configured prefix.
Use the same group only for slots actually sharing quota. Credentials/model names
are not quota identities. A missing binding fails composition explicitly.

Use the same effective config/base directory for the broker and agents:

```sh
quecto admission-broker run
# In another terminal:
quecto admission-broker status
```

`run` stays foreground until SIGTERM/SIGINT and prints
`admission authority ready: <socket>`. A second broker on the directory exits
with status 3. Start the broker before agents; restart agents after configuration
changes (admission policy cannot change by live reload). Fresh status has
`journal_healthy: true`, an epoch, and zero per-group counts.

## Parallel spawn and queue inspection

Admission limits provider attempts, **not agent count**. Spawn independent work
in parallel as usual. Spawning, tools, idle agents, and parents waiting for
children hold no inference permit. Children register a capability before socket
readiness; socket readiness does not mean their first provider attempt is admitted.
An interactive root can use reserved capacity while background children queue.

For example, call `multi_tool_use.parallel` with two independent read-only tasks:

```json
{
  "tool_uses": [
    {
      "recipient_name": "functions.spawn",
      "parameters": {
        "agent_id": "review-config",
        "read_only": true,
        "task": "Review admission configuration and report missing provider bindings."
      }
    },
    {
      "recipient_name": "functions.spawn",
      "parameters": {
        "agent_id": "review-tests",
        "read_only": true,
        "task": "Review admission test coverage and report gaps."
      }
    }
  ]
}
```

These spawn calls can run concurrently; the broker separately admits each
child's inference. Save each returned UUID, not just its display label.
`read_only` disables write/edit tools but is not a sandbox for Bash.

After spawning, do unrelated work or yield; do not poll/sleep/wait-loop. Use
`agent_cmd get_state` occasionally with the spawn-returned UUID for live
supervision. A completion notification is only a hint: then call
`agent_cmd get_messages` with that UUID and no count/before to read the report.
Before replacing an apparently failed child, inspect its existing state.

Queued inference is waiting, not stalled or idle. `get_state` includes `admission`
and `progress.state = "waiting"`; inspect `waiting`, `admitted`,
`longestWaitSeconds`, group `cooldown`/`lastRefusal`, and counters. This is a
process-local bounded view (up to 64 sampled live attempts, extras in `hidden`),
not a global queue position. Full reads refresh elapsed wait/cooldown values;
`admission_state_changed` events describe transitions. An unchanged generation
is not proof that time stopped. `abort` cancels a queued attempt and increments
`counters.cancelled`.

The TUI left-panel status `waiting for admission` means a provider attempt is
queued at the broker, not that the child failed, stopped, or is awaiting a user
prompt. It can reflect the session's own wait or a forwarded descendant wait.
Its wait timer advances using local monotonic elapsed time only while waiting
with a known duration. Each authoritative new snapshot rebases that timer;
a grant clears it. An unknown duration remains waiting without seconds rather
than inventing a start time. This is elapsed queue wait, not an ETA or queue
position. Cooldown display is separate and currently static between snapshots;
its live countdown is tracked in #1708.

For global accounting, run `quecto admission-broker status`: `groups` contains
`active`, `queued`, `uncertain`, `cooldown_until_ms`, and `unavailable` alongside
`epoch`, `journal_healthy`, and `live_scopes`. Cooldown timestamps use the
broker's clock domain, not Unix wall time; prefer session cooldown
`remainingSeconds` for a human-readable remainder.

## Timer and failure troubleshooting

- Free capacity with queued work can be normal: check reserved capacity,
  `min_interval_ms`, shared cooldown, and quarantine before assuming a stall.
- A pacing/cooldown timer should resume eligible queued work without another
  client request; a queue timer should expire waiting at `queue_timeout_ms`.
  If a deadline passes without progress, capture broker status plus a full
  child `get_state` and report a timer/wakeup fault. Repeated status requests,
  extra spawns, or killing children are not a repair strategy.
- Queue-full or queue-timeout refusal means waiting limits were reached, not
  that spawn failed. Reduce concurrent provider demand or have the operator
  adjust measured policy and restart; concurrency/pacing do not guarantee RPM/TPM.
- `journal_healthy: false` prevents safe grants: investigate storage/permissions.
  An unreachable broker requires checking the service, effective directory and
  config. Do not bypass admission to hide the error.
- `uncertain > 0` quarantines a group: disconnected/killed attempts may still
  run remotely. A surviving session reconnecting with the same capability and
  completing its attempt can clear uncertainty. Closing a socket does not
  cancel remote computation; an attempt timeout is not proof of remote completion.
- `unavailable` can mean provider cooldown advice exceeded `max_cooldown_ms`;
  treat this as a provider incident. Ordinary cooldown requires waiting, not reset.

Recovery is an operator decision:

```sh
quecto admission-broker reset
```

Reset starts a new epoch and revokes every capability: restart every session.
It explicitly gives up the claim that old remote work is bounded. Cooldown and
pacing deadlines survive reset. A broker restart preserves journaled occupancy
and turns outstanding work uncertain; existing sessions must restart to register
again. A missing ledger after prior operation is refused;
`run --accept-missing-ledger` explicitly accepts starting empty, not safe recovery
of old remote work. Do not delete the journal to clear a queue.

To disable admission deliberately, remove the whole section and restart all
agents before stopping the broker; removing selected bindings is not a bypass.

Container children require the shared `client/` directory at the same path.
The bundled adapter uses `QUECTO_ADMISSION_DIR` and reports
`admission_capability: shared-directory-v1`; incompatible scripts are refused.
Expose only the client directory, never the owner token, admin socket or journal.
See `docs {"name":"subagents"}` for container spawning.
