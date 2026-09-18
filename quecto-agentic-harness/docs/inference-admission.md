# Shared inference admission (#1679)

Quecto can bound how many provider requests all of one user's cooperating
sessions and sub-agents on one host have in flight at once, with request
pacing, fair queueing and a shared cooldown when a provider throttles. It
limits **outbound provider attempts**, never the number of agents: spawning,
tool execution, idle agents and parents waiting on children hold no permit.

Admission is **disabled by default**. Configuring an `admission` section enables
it; the authority must be running before agents start. An unavailable authority
is an error, not a fallback to unbounded inference.

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
    "bindings": { "openai-oauth": "acct-openai" }
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
- `bindings`: exact router provider slot to alias (see below). Every provider
  constructed by the runtime must have an explicit binding when admission is
  enabled; unknown aliases fail configuration.
- `directory` (optional, default `<base_dir>/admission`): a private, owner-only
  directory holding the authority's lock, journal and sockets. It must be
  absolute and must not be the base directory or one of its ancestors; it is
  refused if group/other bits are set.

### Provider slots and authentication

The example binds **OpenAI OAuth** (`openai-oauth`), not API-key requests. It
is sufficient only when that is the only provider the runtime constructs.
Built-in slots are auth-specific:

| Slot | Authentication |
| --- | --- |
| `openai-oauth` | OpenAI OAuth |
| `openai-api` | OpenAI API key |
| `anthropic-oauth` | Anthropic OAuth |
| `anthropic-api` | Anthropic API key |

Bare `openai` and `anthropic` are not the production built-in router slots;
they do not bind the auth-specific providers. Custom `models.json` providers
use their provider key as the slot (not their `oauthProvider` credential key).
An `openai_compatible` endpoint uses its configured `prefix`.

Bind **all constructed providers**, not just the provider of the selected
model. Available credentials and configured endpoints/registry entries can
cause additional providers to be constructed even when unused by the current
session. For example, if both OpenAI API-key and OAuth providers are
constructed, both `openai-api` and `openai-oauth` need bindings. A missing
binding fails provider composition with
`admission provider '<slot>' requires an explicit alias binding`; it does not
leave that provider unbounded. Providers skipped because their required
credentials are unavailable do not need bindings.

Multiple slots can map to the same alias/group when they share a quota; use
separate groups for independent quotas. Authentication mode alone does not
establish whether accounts share a quota. Adding credentials or another
provider may require adding a binding and restarting.

No numeric value above is a vendor-safe default; measure and set your own.
Policy and bindings are restart-only: a reload with a changed section is
rejected and the live runtime keeps its current values.

## Running the authority

```sh
quecto admission-broker run      # holds the singleton lock until SIGTERM/SIGINT
quecto admission-broker status   # JSON: epoch, journal health, per-group counts
quecto admission-broker reset    # new accounting epoch (see recovery below)
```

### One host-wide broker, agent-operable (#2024 S3)

There is one broker per host/user. Repo configs never carry an `admission`
section (it is global-only), so **children inherit the parent's authority
unconditionally**: a child launched with a different `--config`, or one whose
config sets `"admission": null`, binds the capability its parent registered and
never has to match the published policy (this closes the "restart required"
failure #2023). `validate_candidate`'s byte-equal check now applies only to a
root's own live reload.

`status`, `reset`, `run`, `install-service` and `uninstall-service` address the
**global** config (`<base_dir>/config.json`) or an explicit `--config <path>` /
`--directory <dir>`, never the working-directory overlay, so the working
directory no longer changes which broker you address. Every output names the
directory it addressed.

Install the broker as a systemd *user* service instead of a bare terminal:

```sh
quecto admission-broker install-service --config ~/.quecto/config.json  # writes the unit, daemon-reload, enable --now
quecto admission-broker status                                          # verify: directory, epoch, journal_healthy
quecto admission-broker uninstall-service                              # disable --now and remove the unit
```

Both are idempotent and report exactly what they did; a changed unit is
rewritten and the service restarted on it; `--dry-run` prints the plan without
touching systemd. The unit runs `admission-broker run --config "<abs config>"`
(paths quoted) with `After=basic.target` (never `After=default.target`, which
would cycle with `WantedBy=default.target` and drop the job at login),
`Restart=on-failure`, and `RestartPreventExitStatus=3` so a `Busy` exit (another
broker holds the lock) does not loop.

A **default binding** — a `"*"` (or `"default"`) key in `bindings` — catches
every provider slot with no explicit binding, so adding a provider does not fail
composition. Its alias must still exist; an explicit slot binding wins over it.

**Roots survive a broker restart or reset without restarting; children fail
closed and are respawned.** Every gate of a process routes through one
`AuthorityLink`. A loss is a closed socket (broker died) *or* a revoked
capability (`reset` keeps sockets open and revokes every credential). A root
link re-registers on its next attempt or spawn with the owner token currently
on disk, with bounded jittered backoff (8 tries, ≈10 s); `get_state.admission`
and every pushed `admission_state_changed` carry `authorityStatus` live:
`reconnecting` from the moment of the loss, `connected` once re-registered
(`epoch` moves after a reset), `unavailable` when reconnection was exhausted
(retried on the next attempt) or when the broker came back publishing a
**different policy** — the session composed against the old one, so it fails
closed with "restart required" permanently. An attempt in flight at the loss
fails closed exactly once with a clear `admission: …` error; nothing ever runs
unadmitted. A child link has no reconnect: its `authorityStatus` becomes
`unavailable` and its next attempt fails closed once
(`admission refused: capability revoked by an authority reset; a child cannot
re-register on its own — its parent must respawn it`, or
`admission authority connection closed` after a broker death). "Once" is a
classification rule, not luck, and the rule is exhaustive over every
`admission: …` message the gate can raise — none reaches the generic keyword
classifier, where "authority" would read as an auth failure and the OAuth
decorator would refresh and re-send (a second admission attempt).
`admission refused: …`, `admission capability rejected`,
`admission authority was reset` (an acquire queued across a reset; the next
attempt re-registers), `admission queue wait deadline elapsed`,
`admission ledger not durable`, `admission protocol violation: …`,
`unsupported admission protocol: …` and any other `admission: …` message map
to the terminal `admission` provider-error class (no provider retry, no
malformed-request repair, no credential hint). `admission request cancelled`
is the caller's own `cancelled`. `admission transport failure: …` and
`admission authority connection closed` (the broker died while this attempt
waited) are retryable `network`: the retry is a fresh attempt that acquires
at the gate again over the link's reconnect, never a resend around it. The
`admission` class is additive on the audit wire
(`AuditEvent::ProviderError.class`), like `empty_stream`. The parent sees
the child's `agent_error`, the forwarded `admission_state_changed` with
`authorityStatus: "unavailable"`, and `agent_cmd get_state` with the same plus
`counters.refused`; it spawns a replacement (its own link re-registers first,
minting the new child in the current epoch).

`run` prints `admission authority ready: <socket>` on stderr. A second `run`
on the same directory exits with status 3; `run --directory` must name the
config's own directory or is refused. Stopping the authority does not kill
sessions; capabilities live only in the authority process, so after a restart
roots re-register and children fail closed as above. A ledger that is missing
after prior operation is refused; `run --accept-missing-ledger` explicitly
starts an empty one. Roots are minted with the owner token at
`<directory>/root.token` (0600, outside `client/`), so a process that only sees
the client directory can bind a pre-registered child capability but never
promote itself to a root.

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
  orphaned uncertain occupancy; it never restarts with an empty ledger. Roots
  reconnect and re-register; children fail closed and are respawned.
- `quecto admission-broker reset` starts a new epoch, revokes every
  capability (roots re-register on their next attempt; children fail closed
  once and are respawned by their parents) and explicitly gives up any claim
  that old remote work is bounded. Cooldown and pacing deadlines survive the
  reset.

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

The same runbook the `docs` tool serves as `admission-broker`: preconditions,
commands, expected outputs, rollback, failures.

Preconditions: a systemd *user* session for `install-service` (otherwise
supervise `quecto admission-broker run --config <abs global config>` yourself);
`quecto status` exits 0 and its `Config:` line is the global file the section
goes into (`QUECTO_BASE_DIR` moves it and the default `<base_dir>/admission`
directory, whose path must be short enough for a Unix socket); no other broker
holds the directory (`quecto admission-broker status` → `not running for
directory …`, exit 1, before you start).

Activation (per host, per user):

1. Write the section into the global file (never a repo overlay — refused as
   global-only); with a default binding `"*"` you need not list every
   constructed provider slot. Measure your own values; none of the numbers
   here are vendor-safe defaults.
   ```sh
   quecto config set --global admission '{"groups":{"shared":{"capacity":4,"reserve":1,"min_interval_ms":250,"queue_capacity":64,"queue_timeout_ms":120000,"attempt_timeout_ms":900000,"fallback_base_ms":2000,"max_cooldown_ms":600000}},"aliases":{"account":"shared"},"bindings":{"*":"account"}}'
   ```
   Expected: `set admission in /home/me/.quecto/config.json`, exit 0.
2. Plan, then install the service:
   ```sh
   quecto admission-broker install-service --dry-run
   quecto admission-broker install-service
   ```
   Expected dry run:
   ```
   admission-broker: would apply quecto-admission-broker.service (directory /home/me/.quecto/admission)
     - (dry run) write unit /home/me/.config/systemd/user/quecto-admission-broker.service
     - (dry run) systemctl --user daemon-reload && enable --now quecto-admission-broker.service
   ```
   Expected install: `applied quecto-admission-broker.service (directory …)`
   with `wrote unit …`, `reloaded the user daemon`, `enabled and started …`
   (a second run: `unit … already up to date`). Do not run `quecto
   admission-broker run` from an agent tool call: it is foreground-only.
3. Verify: `quecto admission-broker status` prints
   `{"directory":"/home/me/.quecto/admission","epoch":1,"journal_healthy":true,"live_scopes":0,"groups":{"shared":{"active":0,"queued":0,"uncertain":0,"cooldown_until_ms":0,"unavailable":false}}}`,
   exit 0; `systemctl --user status quecto-admission-broker.service` →
   `active (running)`.
4. Start (or restart) every agent process. Roots register before composing a
   provider, so a session that cannot reach the authority exits with an error
   before any inference; a session started before the section existed keeps
   running unbounded until restarted (a live reload never switches admission
   on or off; it keeps the last-good runtime).
5. Check a session: `get_state` carries `admission` (`directory`, `epoch`,
   `connected`, `authorityStatus`), and a queued attempt shows
   `progress.state = "waiting"` (TUI: "⏳ waiting for admission").

Rollback:

1. `quecto admission-broker uninstall-service` (idempotent; `no unit to
   remove at …` on a second run), then `quecto config unset --global
   admission` (or `config set --global admission null`). Removing bindings
   while keeping admission enabled is not a selective bypass: missing
   bindings for constructed providers cause composition to fail.
2. Restart every agent process; a live reload with a changed section is
   rejected by design, so nothing changes until the restart.
3. Stop the authority (the uninstall did, or SIGTERM a foreground `run`).
   Outstanding remote work is no longer bounded from that moment; rollback
   does not pretend otherwise.

If it fails:

| Symptom | Cause | Fix |
|---|---|---|
| `admission-broker: config … not found; no \`admission\` section to address` / `no \`admission\` section is configured; nothing to address (or pass --directory)` | step 1 not done, or another base dir | `quecto config get --global admission`; check `QUECTO_BASE_DIR` |
| `refusing to write …: \`admission\` is global-only …` | `config set` without `--global` | add `--global` |
| `refusing to write …: the result is not a valid configuration: …` | a group field missing, `reserve >= capacity`, `fallback_base_ms > max_cooldown_ms`, an alias without a group, a binding to an unknown alias | fix the JSON; the file is unchanged |
| `status` → `not running for directory … (admission transport failure: connect …/admin.sock: No such file or directory)`, exit 1 | broker not started, or a different directory than expected | `systemctl --user status quecto-admission-broker.service`; `journalctl --user -u quecto-admission-broker.service -n 50`; `quecto config get --global admission.directory` |
| `… path must be shorter than SUN_LEN` | the admission directory path is too long for a Unix socket | set `admission.directory` to a short absolute path (not the base dir or an ancestor), reinstall |
| `another admission authority owns …/authority.lock`, exit 3 (not restarted: `RestartPreventExitStatus=3`) | a foreground `run` or an old service holds the lock | stop it, `systemctl --user restart quecto-admission-broker.service` |
| `install-service` → `systemctl` errors | no systemd user session | `loginctl enable-linger $USER`, or supervise `run --config <abs>` yourself |
| an agent exits with `admission negotiation failed: admission authority unreachable at …` | section present, broker down | start the service, or roll back |
| a session reports "restart required" | its own section changed on live reload, or the broker came back with a different policy | restart that session (never a child: children inherit) |
| `journal_healthy: false` | the journal is unwritable or corrupt | fix permissions/storage; never delete the journal to clear a queue |

Quarantine (a group has stopped granting):

1. `quecto admission-broker status` shows `uncertain > 0` for the group: a
   session disconnected or was killed while an attempt was outstanding, or the
   authority restarted with work in flight.
2. If that session is still alive and reconnects with the same capability,
   completing the attempt clears the entry; wait for it.
3. Otherwise `quecto admission-broker reset` starts a new epoch. It revokes
   every capability — roots re-register on their next prompt without an agent
   restart, children fail closed once and must be respawned by their parents —
   and it gives up the claim that the old remote work is bounded. Cooldown and
   pacing deadlines survive.
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
