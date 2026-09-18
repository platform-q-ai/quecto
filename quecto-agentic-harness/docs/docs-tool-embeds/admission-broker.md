# Admission broker

Shared inference admission bounds outbound provider attempts across cooperating
sessions and subagents for one user on one host. It is **disabled by default**:
omitting `admission` leaves inference unbounded by this broker. When configured,
an unreachable authority is an error, never an unbounded fallback.

## Configuration reference

The runbook (Preconditions → Do → Verify → Rollback → If it fails) is below;
this section explains the fields. Example fragment (illustrative values, not vendor-safe defaults):

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
are not quota identities. A missing binding fails composition explicitly, unless
you add a **default binding**: a `"*"` (or `"default"`) key whose alias catches
every unlisted slot, so adding a provider does not fail composition for want of a
new binding. The default alias must itself exist. Example
`"bindings": { "*": "account" }`. An explicit slot binding still wins over it.

## One host-wide broker, agent-operable (#2024 S3)

There is exactly one broker per host/user; repo configs never carry an
`admission` section (it is global-only), so children inherit the parent's
authority unconditionally — a child launched with a different or `"admission":
null` config binds the capability its parent registered and never has to match
the published policy. `admission-broker status`, `reset`, `run`,
`install-service` and `uninstall-service` address the **global** config
(`<base_dir>/config.json`) or an explicit `--config <path>` / `--directory
<dir>`, never the working-directory overlay, so the cwd does not change which
broker you address. Every output names the directory it addressed.

## Preconditions

- Linux with a systemd *user* session (`systemctl --user status` answers) for `install-service`; without one, run the broker under your own supervisor with `quecto admission-broker run --config <abs global config>`.
- `quecto status` exits 0; its `Config:` line is the global file the section goes into (`QUECTO_BASE_DIR` moves it and the default `<base_dir>/admission` directory). The directory path must be short enough for a Unix socket (under ~100 characters).
- No other broker holds `<base_dir>/admission` (a foreground `quecto admission-broker run` in a terminal, say): `quecto admission-broker status --directory <base_dir>/admission` → `not running for directory …`, exit 1, before you start. (Without `--directory` and before step 1, `status` says `config … not found; no `+'`admission`'+` section to address` or `no `+'`admission`'+` section is configured …` — also "no broker addressed".)
- You know which provider slots the runtime constructs; with a default binding (`"*"`) you need not list them.

## Do

1. **Write the section** into the global file (values are illustrative — measure your own; every group field is required):
   ```
   quecto config set --global admission '{"groups":{"shared":{"capacity":4,"reserve":1,"min_interval_ms":250,"queue_capacity":64,"queue_timeout_ms":120000,"attempt_timeout_ms":900000,"fallback_base_ms":2000,"max_cooldown_ms":600000}},"aliases":{"account":"shared"},"bindings":{"*":"account"}}'
   ```
   Expected: `set admission in /home/me/.quecto/config.json` (`(created)` appended when the file did not exist), exit 0. Never write it into a repo overlay (refused: global-only).
2. **Plan** the service install — stop here and show the plan when the user asked only for a plan or a dry run:
   ```
   quecto admission-broker install-service --dry-run
   ```
   Expected dry run:
   ```
   admission-broker: would apply quecto-admission-broker.service (directory /home/me/.quecto/admission)
     - (dry run) write unit /home/me/.config/systemd/user/quecto-admission-broker.service
     - (dry run) systemctl --user daemon-reload && enable --now quecto-admission-broker.service
   ```
3. **Install** (systemd user unit `quecto-admission-broker.service`: `ExecStart=… admission-broker run --config "<abs>"`, `After=basic.target`, `Restart=on-failure`, `RestartPreventExitStatus=3`, `WantedBy=default.target`):
   ```
   quecto admission-broker install-service
   ```
   Expected: `applied quecto-admission-broker.service (directory …/admission)` with "wrote unit …", "reloaded the user daemon", "enabled and started …". Idempotent: a second run reports "already up to date"; a changed binary or config path rewrites the unit and reports "restarted … on the rewritten unit". `--config <abs path>` pins another global file. Do not run `quecto admission-broker run` from a tool call: it stays in the foreground and ends with the call.
4. **Restart agents** you want bounded: sessions started before the section existed compose without admission and keep running unbounded until restarted (a live reload never switches admission on or off — it reports the change and keeps the last-good runtime). Say this in your report; it includes the session you are running in.

## Verify

```
quecto admission-broker status
```

Expected: `{"directory":"/home/me/.quecto/admission","epoch":1,"journal_healthy":true,"live_scopes":0,"groups":{"shared":{"active":0,"queued":0,"uncertain":0,"cooldown_until_ms":0,"unavailable":false}}}`, exit 0 (`epoch` grows by one per `reset`; `live_scopes` counts registered sessions). Then `systemctl --user status quecto-admission-broker.service` → `active (running)`. Optionally (one paid model call, for an operator at a terminal rather than from inside a session) a fresh `quecto agent --no-session -m "Reply with exactly OK"` prints `OK`, and `status` shows `live_scopes` back to 0 after it exits. In a session, `get_state.admission` carries `directory`, `epoch`, `connected: true`, `authorityStatus: "connected"`; the TUI footer shows the broker health.

## Rollback

```
quecto admission-broker uninstall-service        # → applied … "disabled and stopped …", "removed unit …" (second run: "no unit to remove at …")
quecto config unset --global admission           # → unset admission in /home/me/.quecto/config.json
```

Then restart agents (they composed against the policy). Order matters: remove the section and restart agents *before* stopping the broker if sessions must stay admitted until the end; removing selected bindings is not a bypass. `quecto config set --global admission null` also disables (the key stays, as null).

## If it fails

| Symptom | Cause | Fix |
|---|---|---|
| ``admission-broker: config … not found; no `admission` section to address`` / ``no `admission` section is configured; nothing to address (or pass --directory)`` | step 1 not done, or another base dir | `quecto config get --global admission`; check `QUECTO_BASE_DIR` |
| ``cannot change `admission` in …/.quecto/config.json: `admission` is global-only; use --global`` | `config set` without `--global` | add `--global` |
| `refusing to write …: the result is not a valid configuration: …` | a group field missing, `reserve >= capacity`, `fallback_base_ms > max_cooldown_ms`, an alias without a group, a binding to an unknown alias | fix the JSON; the file is unchanged |
| `status` → `not running for directory … (admission transport failure: connect …/admin.sock: No such file or directory)`, exit 1 | broker not started, or a different directory than the one you expect | `systemctl --user status quecto-admission-broker.service`; `journalctl --user -u quecto-admission-broker.service -n 50`; the directory addressed is in the error line itself (`quecto config get --global admission.directory` answers only when you set it explicitly; unset = `<base_dir>/admission`) |
| `… path must be shorter than SUN_LEN` | the admission directory path is too long for a Unix socket | set `admission.directory` to a short absolute path outside the base dir's ancestors, reinstall |
| `another admission authority owns …/authority.lock`, exit 3 (service not restarted: `RestartPreventExitStatus=3`) | a foreground `run` or an old service holds the lock | stop it, then `systemctl --user restart quecto-admission-broker.service` |
| `install-service` → `systemctl` errors | no systemd user session (container, ssh without lingering) | `loginctl enable-linger $USER`, or supervise `quecto admission-broker run --config <abs>` yourself |
| a new agent exits with ``admission negotiation failed: admission authority unreachable at …/client/admission.sock … start `quecto admission-broker run` or remove the admission section`` | section present, broker down | start the service, or roll back |
| a session reports "restart required" | a root's own section changed on live reload, or the broker came back with a different policy | restart that session (never a child: children inherit) |
| `journal_healthy: false` | the journal file is unwritable or corrupt | fix permissions/storage under the directory; never delete the journal to clear a queue |

Recovery from a stuck or uncertain group is `quecto admission-broker reset` → `{"directory":…,"epoch":N}` plus a stderr warning; roots keep running, children are respawned — see below.

## Recovery: reset and broker restart (what each process does, what you see)

A **root session** (a top-level `quecto agent`) survives both a broker restart
and a `reset` without being restarted. Its link notices the loss at once —
`get_state.admission` and the pushed `admission_state_changed` carry
`authorityStatus: "reconnecting"` (TUI footer `admission ⟳`) — and its next
prompt (or spawn) re-registers with the owner token on disk, with bounded,
jittered backoff (8 tries, about ten seconds), then runs: `authorityStatus:
"connected"`, and after a reset `epoch` is the new one. An attempt that was
in flight when the broker went away fails closed exactly once with an
`admission: …` provider error; nothing runs unadmitted. Two ways a root stays
down: reconnection exhausted (broker still not back: `authorityStatus:
"unavailable"`, retried on the next attempt), or the broker came back with a
**different policy** — the session composed against the old one, so it fails
closed with "restart required" and stays `unavailable` until the agent is
restarted.

A **child** (spawned by a parent, bound to a parent-registered capability)
cannot re-register on its own. After a `reset` its `authorityStatus` is
`"unavailable"` and its next prompt fails closed once with
`admission refused: capability revoked by an authority reset; a child cannot
re-register on its own — its parent must respawn it` (after a broker death:
`admission authority connection closed`). It is never silently admitted.

What the **parent** sees, and does: the child's turn ends with an
`agent_error` carrying that message (visible through `agent_cmd get_messages`
and on the child's event stream); the child's forwarded
`admission_state_changed` and `agent_cmd get_state <uuid>` show
`admission.authorityStatus: "unavailable"` with `counters.refused` incremented.
Spawn a replacement for that task — the parent's own link re-registers first,
so the new child is minted a capability in the current epoch — and do not
re-prompt the old child. Roots and the broker itself need nothing.

Foreground alternative (a terminal or your own supervisor, not a tool call):

```sh
quecto admission-broker run          # → admission authority ready: <dir>/client/admission.sock ; stays until SIGTERM/SIGINT
quecto admission-broker status       # from another terminal
```

A second broker on the directory exits with status 3. `run --directory <dir>`
is accepted only when `<dir>` is the directory the config names. Start the
broker before agents; restart agents after policy changes (admission policy
cannot change by live reload; a broker restarted with a changed policy makes
running roots fail closed with "restart required").

## Parallel spawn and queue inspection

Admission limits provider attempts, **not agent count**. Spawn independent work
in parallel as usual. Spawning, tools, idle agents, and parents waiting for
children hold no inference permit. Children register a capability before socket
readiness; socket readiness does not mean their first provider attempt is admitted.
An interactive root can use reserved capacity while background children queue.

For example, two independent read-only tasks in one turn — on an OpenAI model through its `multi_tool_use.parallel` wrapper (shown), on any other provider simply two `spawn` calls in the same turn:

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

Reset starts a new epoch and revokes every capability. It explicitly gives up
the claim that old remote work is bounded. Cooldown and pacing deadlines survive
reset. Roots re-register on their next attempt; children fail closed once and
are respawned by their parents (see "Recovery: reset and broker restart"). A
broker restart preserves journaled occupancy and turns outstanding work
uncertain; roots reconnect the same way. A missing ledger after prior operation
is refused; `run --accept-missing-ledger` explicitly accepts starting empty, not
safe recovery of old remote work. Do not delete the journal to clear a queue.

To disable admission deliberately, remove the whole section and restart all
agents before stopping the broker; removing selected bindings is not a bypass.

Container children require the shared `client/` directory at the same path.
The bundled adapter uses `QUECTO_ADMISSION_DIR` and reports
`admission_capability: shared-directory-v1`; incompatible scripts are refused.
Expose only the client directory, never the owner token, admin socket or journal.
See `docs {"name":"subagents"}` for container spawning.
