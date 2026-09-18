# Container runtimes for subagents (`container_configs`)

Quecto can spawn a subagent into an isolated, script-managed environment
(e.g. a container) instead of a local child process. Quecto itself is
runtime-agnostic: it invokes an executable you configure, and that
executable owns Docker/Podman/devcontainer/whatever details. This is the
single canonical user document for the feature; the single canonical
reference runtime lives at [`scripts/container-runtime/`](../scripts/container-runtime/)
(`create.sh`, `exec.sh`, `inspect.sh`, `kill.sh` — see
"[The canonical reference runtime](#the-canonical-reference-runtime)").

## Spawning

The `spawn` tool's `container` field selects the launch adapter:

| `container` value | Behavior |
| --- | --- |
| omitted or `false` | Local child process (default, unchanged) |
| `true` | New container via the config labeled `"default": true` |
| `{"mode": "new", "container_config"?: "...", "name"?: "..."}` | New container via the named config, with an optional container name |
| `{"mode": "existing", "ref": "C1"}` | Join the existing session environment `C1` via its retained `exec` script |
| `{"mode": "existing", "name": "review-env"}` | Join an existing environment by its (unambiguous) name |

Unknown fields are rejected. Runtime-specific fields (`branch`, `pr`,
`image`, ...) do not exist. `mode: existing` requires exactly one of
`ref`/`name`; unknown, ambiguous, stopped, or stale (kill pending/failed)
targets fail without guessing.

- There is **no `repo` field** (#1410): a container config is a complete,
  self-contained definition of a working context — its repository URL and any
  auth it needs are baked into the config's own argv, and the parent's
  location or checkout is irrelevant. A config with no repository is a
  **sandbox**: empty workspace, fully valid.
- `container_configs` are read from the launching agent's **effective
  configuration** (#2024): its global file with the trusted repo-local
  overlay of its working directory merged in, resolved fresh at every spawn
  — so container spawns normally need no `config` argument. An explicit
  `config` argument in the spawn call replaces both layers (as `--config`
  does) and must be an absolute path. To find the names in effect run
  `quecto config get --effective container_configs`.

A successful spawn returns a session-scoped environment reference
(`environment_ref=C1`, `C2`, ...). Refs are minted once per session and
never reused — a stopped environment stays listed and its ref is retired.
The child then behaves like any other subagent: drive it with normal
`agent_cmd` operations over its direct or proxy endpoint. Every member of an
environment shares its reported workspace; each agent keeps its own agent
UUID, distinct from the environment's hidden UUID.

## Listing and killing environments (`agent_cmd`)

The session environment registry is authoritative for `CN` ref, optional
name, runtime id, repository, workspace, retained script set, member agent
UUIDs, status, metadata, and last error. Two session-level `agent_cmd`
commands expose it (use `agent_id: "*"`):

- `get_containers` — lists every environment this session committed with
  status `running`, `empty` (live, no members), `killing`, `stopped`,
  `cleanup-failed` (with its `last_error`), or `retained` (a swarm
  container kept alive after its run ended or lost its coordinator, with
  `metadata.retained` explaining which; see "Swarm environments are
  retained"), plus workspace and members.
- `kill_container` with `ref` or `name` — takes the environment's
  exclusive kill claim, asks every member agent to shut down over its own
  control edge (the `shutdown` protocol; a container coordinator's harness
  settles its in-container descendants itself, and the host never signals
  a pid inside the box), then runs the environment's retained `kill` argv
  exactly once and commits `stopped` only after the script succeeds. Its
  JSON result includes the environment ref, up to 20 member agent
  ids/names (with `omitted_agents` when capped) and `settled`, how each
  member ended before the kill ran (`graceful`, `fallback`,
  `already-exited`, `unobserved` — asked, but no exit was observed within
  the bound, so the retained `kill` is what ends it — or `joined`). A
  member whose end cannot be settled (a termination another path already
  owns that never settles, or a locally owned process that survives the
  fallback) withholds the retained `kill` and persists a retryable
  `cleanup-failed` state naming the member; a failed `kill` script does the
  same. Run `kill_container` again to retry: only the members still
  recorded are asked again, and the retained `kill` never runs twice under
  one claim. Latency: a member is asked over its edge with a 5 s
  acknowledgement bound and, once acknowledged, given up to 25 s (the
  owned-handle ladder plus slack) for its exit to be observed before it is
  compensated `unobserved`, so a `kill_container` whose members are
  unreachable or slow to exit can take up to ~30 s per member before the
  retained `kill` runs; a member that exits promptly settles in
  milliseconds.

When the final member of a live environment exits or is killed, the same
retained `kill` operation runs exactly once (concurrent final exits cannot
double-kill). Script sets without a configured `kill` fall back to the
retained `cleanup` argv for final-member teardown; `kill_container` itself
refuses such environments up front, leaving every member untouched.

### No host signal ever enters a container

A container member's harness — and everything it launches inside the box —
is ended over the protocol only: the host asks it to `shutdown`, observes
the exit through its snapshots, and lets the retained `kill` argv end the
box. The host holds no process handle for anything inside a container and
records no pid for a descendant reported from inside one (a merged row's pid
is always `0`), so pids 1 and 2 inside a box — `podman-init` and the
coordinator — can never be targeted from outside. This is ratcheted by
`quecto-agentic-harness/tests/architecture/teardown_authority.rs` (the
process-effect allowlist) and proven by running the harness's own full BDD
suite inside a `quecto-box:local` container whose pid 2 blocks and logs every
signal it receives (see the #1940 record below).

#### In-container proof record (#1925 method, #1940 run)

Method (issue #1925): the harness's own full BDD suite runs inside a
`quecto-box:local` container as the child of a **signal-logging pid 2** — a
Python wrapper that blocks `SIGTERM`/`SIGINT`/`SIGHUP`, runs the suite as its
child (with the mask unblocked for the child), and logs every signal it
receives from a `sigwaitinfo` loop with `si_pid` and the sender's `cmdline`.
Pid 2 is where a swarm coordinator's harness sits, so any registry, fixture or
descendant pid the suite ever targeted would receive the signal there.

- **Revision:** `ec29e9902b96fdab2534a0f51dd82bdbace0f5e2` (the #1940 PR head at the time of the run; the commits that record it follow)
- **Image:** `quecto-box:local`, id `b1f87e8917502e1963979a0ed43fd7866427c961c26aac0d1576a17774b63662`
- **Command:** `scripts/bdd-in-box/run.sh` (committed with the pid 2
  wrapper `scripts/bdd-in-box/pid2_signal_log.py`), which runs exactly:

  ```bash
  podman run --rm --init --name quecto-bdd-in-box \
    --userns=keep-id --pids-limit 16384 --user 1000:1000 \
    -v <repo>:/src -v quecto-bdd-in-box-target:/tmp/target \
    -v <scratch>/home:/home/dev -v scripts/bdd-in-box/pid2_signal_log.py:/pid2_signal_log.py:ro \
    -e CARGO_TARGET_DIR=/tmp/target -e HOME=/home/dev -e TMPDIR=/home/dev/tmp \
    -e PID2_SIGNAL_LOG=/home/dev/pid2-signals.log -e RUST_LOG=warn -w /src \
    quecto-box:local python3 /pid2_signal_log.py \
    bash -c 'status=0; for i in 0 1 2 3; do echo "=== shard $i/4 ==="; \
      QUECTO_BDD_SHARD_INDEX=$i QUECTO_BDD_SHARD_TOTAL=4 \
      cargo test --workspace --features quecto-agentic-harness/test-support --bins --test bdd || status=1; done; exit $status'
  ```

  The suite runs as four sequential shards (one `bdd` process each) under
  the same pid 2 because one process running all ~1600 scenarios exhausts the
  container's 16384-pid cgroup: the BDD steps leak forgotten tokio runtimes
  (`std::mem::forget(runtime)`, 128 sites), ~19 threads per scenario, and a
  single-process run stalled at 16378 threads with `Cannot fork` — tracked
  by #1959, whose close re-runs the proof as one process.
- **Log:** `/var/tmp/q1940/proof/final-run.log` (10 700 lines) and
  `/var/tmp/q1940/proof/final-pid2-signals.log` on the machine that ran it.
- **Counts:** 4 shards, **1609 scenarios passed, 0 failed** (393 + 376 + 394
  + 446), 8021 steps passed; child exit status 0; runtime 15:18:26 →
  15:26:41 (+ a 4 min cold build).
- **Signals to pid 2: 0** (`SIGNALS_TO_PID2 0`); the wrapper survived with
  pid 2 for the whole run.

Found by this proof and fixed in the same PR: a launched child whose launcher
died and whose stderr pipe therefore had no reader hung on its first log line
when `RUST_LOG` was set (`tracing-subscriber` reports a failed write with
`eprintln!`, which panics on a broken stderr inside the task that was about
to run the parent-loss shutdown). `RedactingWriter` now swallows sink errors
and the subscriber's internal-error reporting is off; the
`subagent_delegated_subtree.feature` SIGKILL scenario reproduces the
configuration on the host and fails without the fix.

### Swarm environments are retained (#1924)

The one exception to final-member teardown is a swarm container: it is kept
after **every** swarm end — ended, closed, cancelled, or lost — including a
fully successful run, because the full end state of a swarm (board,
checkout, unpushed branches, member logs) is worth inspecting. Whenever the
member whose exit empties the environment was the coordinator of a created
run (any status), the cascade still marks the member (and its descendants)
exited, but the environment record moves to `retained` instead of running
the retained `kill`; the container and its state directory stay, and only
an explicit `kill_container` from the host master tears them down
(`swarm_control close` only makes the held outcome terminal; it never
removes the container). That holds for the coordinator's own socket closing,
for an explicit `agent_cmd kill` of the coordinator agent and for the
master's own process shutdown alike. `metadata.retained` says which case
applies:

- **Orderly end** — the run was already paused holding an outcome, closed
  into it, or cancelled when the member went away: `run ended: <outcome>;
  environment retained for inspection, kill_container to remove`, `run
  closed: <outcome>; ...` or `run ended: cancelled; ...`. The run is
  untouched: no quarantine and no `resume_blockers` entry.
- **Loss** — the run was `running`, or paused with no outcome, when the
  coordinator's socket closed: the coordinator is quarantined exactly as an
  in-swarm reconcile treats a lost harness, the run is paused holding
  `failed`, and the control receipt's `resume_blockers` names the
  coordinator that must be relaunched before a resume can proceed;
  `metadata.retained` reads `swarm coordinator '<id>' lost its connection;
  run paused holding failed; ...`. Observation and quarantine are one store
  operation, so a run that ends first (for instance the store's own expiry
  check ending it as `budget-exhausted`) is reported as an orderly end.
- **Supervisor kill** — `agent_cmd kill` of the coordinator, or the master's
  shutdown: `coordinator killed by supervisor; run <status>; ...`. Nothing is
  quarantined; the run is left as it was.

The decision is made from the session that launched the container by
reading the coordination store at `metadata.checkout` (see the `create`
contract). A create result that omits `metadata.checkout` (older or
third-party script sets) is warned about at create time and the host then
probes `<workspace>/repo` and `<workspace>` for a store. Either way the
path must be absolute, `..`-free and — after resolving symlinks on both
sides — under the environment's workspace; anything else is ignored, as if
no store existed. An environment without a store, or whose run is the
bootstrap placeholder, keeps the ordinary final-member teardown, while a
store that exists but cannot be read (contended, corrupt) also retains — a
destroyed box cannot be recovered, a retained one can always be killed.

A retained environment is for inspection: exec into the container, read the
board and checkout, read the member logs (see the adapter section). A join
(`{"mode":"existing","ref":"C1"}`) is admitted but does not revive the
environment (relaunching the coordinator against the surviving store is not
wired yet, and the paused run refuses activation); a joiner that exits or
rolls back leaves the record `retained`. A retained environment outlives the
master session that produced it (the master's shutdown retains it too):
remove it with `kill_container` from that session while it lives, from a new
session that joins and kills it, or manually with the adapter's `kill.sh`
(or `podman rm -f quecto-<environment_id>` plus its state directory).

## Configuration

> **Migrating from pre-#1410 configs:** the `container_scripts` key was
> renamed to `container_configs` and its shape changed (flat map of named
> container configs; the default is labeled `"default": true` on one entry;
> repositories are baked into each config's `create` argv via `--repo`).
> A config still containing `container_scripts` fails to load with an error
> pointing here — rename and reshape the section.

```json
{
  "container_configs": {
    "quecto": {
      "default": true,
      "create": ["/abs/path/to/create-script", "--repo", "https://github.com/platform-q-ai/quecto"],
      "cleanup": ["/abs/path/to/cleanup-script"],
      "exec": ["/abs/path/to/exec-script"],
      "kill": ["/abs/path/to/kill-script"],
      "inspect": ["/abs/path/to/inspect-script"]
    },
    "sandbox": {
      "create": ["/abs/path/to/create-script"],
      "cleanup": ["/abs/path/to/cleanup-script"]
    }
  }
}
```

Each entry is a named **container config**: a complete, self-contained
definition of a working context. Exactly one entry must carry
`"default": true` — the config `container: true` selects; zero or multiple
default labels fail at config **load** time with an error naming the
configured entries. Operations are argv arrays executed directly — no
shell interpolation — and the repository (with any auth it needs) is part
of the config's own argv (`--repo` for the shipped scripts), never
something Quecto resolves or passes. `create` and `cleanup` are required;
`exec` (joining), `kill` (explicit stop), and `inspect` (post-mortem) are
optional but needed for `mode: existing`, `kill_container`, and death
diagnostics respectively. Missing, unknown, empty required, or unsafe
(empty/NUL argument) configuration fails before any script runs, and
selection errors enumerate the available config names so an agent can
offer the menu. Agents see the names before spawning in two places (#2024
S4c): the `spawn` tool description carries one bounded roster line
(`Available container configs: <name> (default, repo-bound|global), …`,
at most 120 characters, the tail folded into `+N more`; a withheld overlay
adds `(repo overlay untrusted — run quecto config trust)`; one entry alone
over the budget is cut with an ellipsis), rendered whenever the tool
definitions are built (registration and every rebuild); and
`agent_cmd {"agent_id":"*","command":"get_container_configs"}` returns the
same effective set with detail, live at each call:
`{"container_configs":[{"name","default","source":"overlay"|"global","repository","problem"}],"overlay_withheld":bool,"diagnostics":[…]}`
— the `container: true` default first; `default` is what a launch would
honour (none while the overlay is withheld, none when more than one entry
is labelled, never an entry with a `problem` — a missing or unsafe argv,
diagnosed in `diagnostics`); `source` says which layer declared the entry,
`repository` is the create argv's `--repo` (`null` for a sandbox). Operators
see the same set with `quecto config get --effective container_configs`.

The container config in effect when an environment is **created** is
retained with the environment: later joins, kills, and inspects use the
retained `exec`/`kill`/`inspect` argv even if the labeled default changes
afterwards.

### Binding a repository to a container config

A repository says "my containers use config X" through its repo-local
overlay `<checkout>/.quecto/config.json` — the same overlay every other
section uses (see the `config` docs page and the harness README), with the
same `container_configs` shape. The overlay merges **entry-wise** over the
global file: a local entry with `"default": true` un-defaults every global
entry, a same-name local entry shadows the global one, and other global
entries stay selectable by name. The merged set must still carry exactly
one default; a write that would break that is refused before it lands.

**Preconditions**: a script set with absolute paths (the shipped adapter or
your own), and the repository URL the container should clone.

**Do** — from the checkout (writes the overlay and records its trust):

```
quecto config set --local container_configs.app '{"default":true,"create":["/abs/create.sh","--state-dir","/abs/state","--repo","https://github.com/org/app"],"cleanup":["/abs/cleanup.sh"],"exec":["/abs/exec.sh","--state-dir","/abs/state"],"kill":["/abs/kill.sh","--state-dir","/abs/state"],"inspect":["/abs/inspect.sh","--state-dir","/abs/state"]}'
```

Then, from an agent started in that checkout, `spawn` with
`container: true` creates the container with `app`'s `create` argv; from
any other directory the global default still applies. The binding applies
only to runs started *without* `--config` (an explicit `--config` file
replaces both layers, as a spawn `config` argument does) and never inside
a container child: the child is started with the global file and has no
checkout overlay to bind, so a container spawned from inside a container
selects from the global entries.

**Verify**: `quecto config get --effective container_configs` shows `app`
as the one default; `quecto status` shows `Overlay: … (trusted)`; a spawn
result names `environment_ref=C1 container_config=app` and
`agent_cmd get_containers` lists the repository the create script reported.

**Rollback**: `quecto config unset --local container_configs.app`, or
delete `.quecto/config.json` to drop the whole overlay.

**Trust**: the overlay is applied only when its exact content is recorded
in `<base_dir>/config-overlay-trust.json` (canonical path + SHA-256).
`quecto config set` records trust for what it writes; an overlay written
by hand, or committed by someone else, needs `quecto config trust` from
the checkout after review — an explicit, non-interactive command an agent
can run. Until then the overlay contributes nothing to container spawns.
`container: true` is **refused** when the withheld overlay could have
changed the default — it is a symbolic link, unparseable or failing the trust checks, or an untrusted
document that declares `container_configs` (the default it labels is
unknown, so an implicit selection must not quietly land in the global
one) — with a tool error naming the overlay and `quecto config trust`. An
untrusted overlay that declares no `container_configs` (one that only pins
`agents.defaults.model`, say) cannot have changed the set: the global
default launches and the result carries the diagnostic as a warning. A
named `container_config` launches from the global set and its result
carries the same diagnostic under `Configuration diagnostics:`; a name
only the withheld overlay defines is `unknown container config` with the
diagnostic appended to the error. The spawn also prints the diagnostic to
stderr, as `quecto status` does. There is
no separate container trust record and no `[y/N]` prompt on the spawn
path any more (the pre-#2024 `container-config-trust.json` is not read;
approve such an overlay once with `quecto config trust`). Container
repository and auth semantics remain self-contained in the selected
config's argv; the parent's checkout supplies the *selection*, never the
source.

## Endpoints and liveness (direct vs proxy)

A `create`/`exec` result must carry **exactly one** endpoint:

- `"socket_path": "/path/to/child.sock"` — a direct UDS endpoint the
  parent connects to; or
- `"socket_proxy": {"argv": ["/abs/path/to/proxy", "args"...]}` — a
  validated argv the parent runs once per connection. The proxy process
  speaks the child protocol on its stdio (typically bridging to a socket
  that is only reachable inside the environment). Unknown keys, an empty
  argv, or empty/NUL arguments are rejected.

Results carrying both or neither endpoint fail the launch with rollback.
In proxy mode Quecto binds a private parent-side bridge socket and never
connects to (or falls back to) the direct socket path that was passed in
the child's CLI args. A bridged connection is torn down — including its
proxy process — as soon as either side closes, so dropped probe or
one-shot command connections never leave proxies (or their connections
into the child) lingering.

Proxy readiness probes the bridge across the launch's readiness budget:
a probe that reads EOF (the proxy could not reach the child yet) is
retried; the endpoint is bridge-ready once a probe survives a quiet
window. For proxy launches with an initial task, an initial prompt send
failure is folded back into the same launch readiness retry budget before
rollback, so a proxy that connected before the child accepted commands
gets another chance without ever falling back to a direct socket or
polling lifecycle state after launch. Because the child protocol cannot
prove whether an ambiguous lost acceptance response was observed after the
child queued the prompt, proxy-managed initial tasks must remain
idempotent: a retry may resend the same initial task if delivery succeeded
but the acknowledgement did not return before the retry deadline.

After readiness the parent holds one persistent monitor connection to the
endpoint. EOF or connection reset on it IS the child's death signal — no
lifecycle polling, no wrapper process. That same connection is the child's
**launch-bound parent control connection** (#1935): the parent presents the
launch capability as its first frame and the child treats the loss of that
one connection as "my parent is gone" and shuts itself down. For this to
hold end to end through a proxy, the proxy argv must exit (closing its
connection into the child) when its stdin reaches EOF — the parent's side of
that pipe closes whenever the parent dies, SIGKILL included. Every proxy
process is owned by the parent's child supervisor; when the bridged
connection ends its stdin is closed first and it receives a signal only if it
does not exit by itself. On death, the environment's
retained `inspect` runs exactly once for that member (repeated EOF/reset
signals do not re-run it), the authoritative environment record is
updated before the member is removed, and the exit feeds passive
completion notes, snapshots, and live events exactly like a local child. Because no exit status exists for a
script-managed death, lifecycle observers report how the death was
observed — `connection_closed` (EOF or reset) or `never_reachable` (the
endpoint never accepted the monitor
connection) — never a fabricated `exit_code_0`.

## Script contract

### `create`

Invoked as:

```
create-argv... -- <child-binary> <child-args...>
```

The repository is part of `create-argv` itself — the shipped scripts take
`--repo <url>` there, and a config that omits it is a sandbox (empty
workspace). Quecto passes **no source information**; environment variables
provided to the script:

| Variable | Meaning |
| --- | --- |
| `QUECTO_CONTAINER_CONFIG` | Name of the selected container config |
| `QUECTO_CONTAINER_ENVIRONMENT_REF` | The minted session ref (`C1`, ...) |
| `QUECTO_BASE_DIR` | Parent agent's base directory (set only when the parent has one) |

The script must start the child (`<child-binary> <child-args...>`)
**exactly once** — Quecto never starts a local fallback child — and print
exactly one JSON object to stdout:

```json
{
  "environment_id": "runtime-owned-id",
  "workspace_path": "/path/to/workspace",
  "metadata": {},
  "socket_path": "/path/to/child.sock"
}
```

`environment_id`, `workspace_path`, and `metadata` are required, plus
exactly one of `socket_path` or `socket_proxy` (see "Endpoints and
liveness"). Extra JSON data after the object is rejected. A non-zero exit
or an invalid contract fails the launch and rolls back. **Logs and
diagnostics go to stderr**: the last 4 KiB of the script's stderr (with
terminal escapes and control characters removed) are appended to the tool
error after the exit status — `script-managed create failed with status
exit status: 6: … image quecto-box:local is not present …` — and echoed on
the parent harness's stderr, so a `die` message reaches both the model and
the operator (#2024 S4b). The same applies to `exec`, and to the retained
`inspect`, `kill` and `cleanup` scripts (a failed cleanup is logged with
its tail).

A create script may also implement the preflight mode the doctor drives:
invoked with `--preflight-only` appended to its configured argv and **no**
child command after `--`, it evaluates every prerequisite without creating
anything and prints one line per check on stdout,
`status<TAB>check<TAB>detail<TAB>remedy` with `status` one of `ok`,
`warn`, `fail`, exiting non-zero when any check failed. The official
Docker/Podman adapter implements it (see "Troubleshooting"); a script that
does not is reported as such by `quecto container doctor`.

The create result must contain exactly these fields — unknown keys are
rejected. When the config cloned a repository, the script should report it
as `metadata.repository`: that is how `get_containers` listings and the
TUI learn the source truthfully (sandbox configs report none and list an
empty repository). A script set that can host a swarm should also report
`metadata.checkout`: the members' working directory and swarm checkout root
(`QUECTO_SWARM_CHECKOUT`), identity-mounted so the session that launched the
container can read the coordination store at
`<checkout>/.quecto/swarm.sqlite` after the members' sockets are gone and
keep the environment instead of destroying a resumable run (#1924).

### `exec`

Invoked to add another agent to an existing environment:

```
exec-argv... -- <child-binary> <child-args...>
```

Environment variables: `QUECTO_CONTAINER_CONFIG` (the retained container
config's name) and `QUECTO_CONTAINER_ENVIRONMENT_ID` (the runtime `environment_id`
reported by `create` — not the session `C` ref). The script must start the
child inside the existing environment exactly once and print exactly one
JSON object:

```json
{
  "metadata": {},
  "socket_path": "/path/to/joined-child.sock"
}
```

The exec result carries a `metadata` object plus exactly one of
`socket_path` or `socket_proxy` (same endpoint contract as `create`);
unknown keys are rejected.

Known limitation: the exec result carries no process handle, so if a join
fails after the script started the child (socket never ready, registration
refused), Quecto cannot terminate that process individually. It keeps
running inside the environment until the environment's retained `kill`
(or final-member `cleanup` fallback) tears the environment down. Exec
scripts should therefore make the started child exit on its own when its
socket is never connected to.

### `kill`

Invoked with `QUECTO_CONTAINER_ENVIRONMENT_ID` set to the runtime
`environment_id`. Runs exactly once per successful stop — from
`kill_container`, from `delete_all_subagents`, when the final member exits,
or when the parent harness receives SIGTERM/SIGINT. A non-zero exit leaves
the environment in a retryable `cleanup-failed` state with the script's
stderr preserved as the last error.

The signal case matters because an environment's processes live outside the
parent's process group: the TUI's ordinary exit (Ctrl-D) terminates the
harness by signal, and nothing but the harness running this script can reach
the environment. The harness therefore tears down every subagent and
environment on the signal before it exits, and the TUI's exit budget (two
seconds) bounds how long the kill script may take.

### `inspect`

Invoked with `QUECTO_CONTAINER_ENVIRONMENT_ID` set to the runtime
`environment_id`, exactly once per dead member, after the member's death
is pushed (EOF/reset) and before the member is removed from the
environment record. A parent-initiated `kill_container` is not a
post-mortem: members terminated by it are not inspected. The inspect
subprocess is bounded (5s); on timeout it is killed and an inspect
failure is persisted, keeping the retained argv for retry. It must print
exactly one JSON object:

```json
{
  "status": "dead",
  "metadata": {"cause": "oom-killed"}
}
```

`metadata` (required object) is merged over the environment's stored
metadata and becomes visible via `get_containers`; `status` (optional) is
recorded as `inspect_status`. The result is parsed with the same strict
wire rules as `create`/`exec`: exactly these fields — unknown keys,
trailing JSON data, and non-UTF8 output are rejected. A non-zero exit or
invalid contract persists an actionable inspect error on the environment
(surviving later successful cleanup) while keeping the retained `inspect`
argv; a later member's successful inspect supersedes and clears the
stale inspect error (a `cleanup-failed` kill error is never cleared by
an inspect).

### `cleanup`

Invoked with `QUECTO_CONTAINER_ENVIRONMENT_ID` set to the runtime
`environment_id` being destroyed (note: a different identity than the
session `C1` ref the create script received). Runs exactly once when a
launch fails after creation (readiness, registration, or initial-prompt
failure) — even when a `kill` is configured. For script sets without a
configured `kill`, the retained `cleanup` argv also serves as the
final-member teardown fallback.

## The canonical reference runtime

The repository ships one canonical reference runtime — a script set that
implements every operation of the contract above and is executed end to end
by the epic acceptance suite
(`quecto-agentic-harness/tests/features/script_managed_runtime_slice5.feature`)
through the production script adapter and strict parser:

- [`scripts/container-runtime/create.sh`](../scripts/container-runtime/create.sh)
- [`scripts/container-runtime/exec.sh`](../scripts/container-runtime/exec.sh)
- [`scripts/container-runtime/inspect.sh`](../scripts/container-runtime/inspect.sh)
- [`scripts/container-runtime/kill.sh`](../scripts/container-runtime/kill.sh)

### Shared inference admission (#1679)

When the parent runs with an `admission` section, `create` and `exec` receive
`QUECTO_ADMISSION_DIR`: the authority's client directory. A script that can
expose that directory to the child **at the same path** must do so and add
`"admission_capability": "shared-directory-v1"` to its JSON result; the bundled
Docker/Podman adapter bind-mounts the directory and reports the capability
(joins report it only when the environment was created with the same
directory). An admission-enabled parent refuses to launch when the capability
is missing, so a script that cannot expose the directory simply omits the
field and the launch fails before any inference. Nothing else in the contract
changes for parents without admission.


The reference runtime is **host-local**: it needs no Docker and runs
everywhere (including CI). Each script takes `--state-dir <dir>` — a trusted
root under which it keeps one directory per environment (checkout workspace,
recorded child pids, invocation records). The root is created owner-only
(mode 700) and adopted only when owned by the invoking user, and each
environment directory is minted with `mktemp -d` — an unpredictable name
that fails hard rather than reuse (or follow a symlink planted at) an
existing path. `create.sh` clones the repository baked into its own argv
(`--repo <url>`; omit it for a sandbox config with an empty workspace) into
`<state>/<environment_id>/workspace/repo` and starts the child directly on
the host with the checkout (or the sandbox workspace) as its working
directory, so the agent genuinely operates inside its isolated workspace; a
failure after state allocation
(e.g. a failed clone) rolls the partially created environment directory back
so nothing unreachable by `cleanup` is ever leaked. `exec.sh` starts a
joining child in that same checkout;
`inspect.sh` reports whether any recorded child is still alive; `kill.sh`
serves both the `kill` and `cleanup` operations, distinguished by
`--op kill` / `--op cleanup`, and performs trusted-root containment (the
environment directory must resolve under the `--state-dir` root) before any
destructive removal. `kill` keeps the environment's recorded metadata for
the cleanup that follows; `cleanup` is terminal and removes the entire
per-environment directory, so the state root does not grow with every
environment ever created. All scripts fail fast (`set -euo pipefail`), log to
stderr only, encode stdout results with a real JSON encoder (`jq`, which
must be installed), and pass the repository URL as a literal argv element
(never shell-interpolated).

A matching configuration (the `--repo` baked into the create argv makes
this config self-contained; drop it for a sandbox config):

```json
{
  "container_configs": {
    "container-runtime": {
        "default": true,
        "create": ["/repo/scripts/container-runtime/create.sh", "--state-dir", "/var/tmp/quecto-envs", "--repo", "https://github.com/you/project"],
        "exec": ["/repo/scripts/container-runtime/exec.sh", "--state-dir", "/var/tmp/quecto-envs"],
        "inspect": ["/repo/scripts/container-runtime/inspect.sh", "--state-dir", "/var/tmp/quecto-envs"],
        "kill": ["/repo/scripts/container-runtime/kill.sh", "--state-dir", "/var/tmp/quecto-envs", "--op", "kill"],
        "cleanup": ["/repo/scripts/container-runtime/kill.sh", "--state-dir", "/var/tmp/quecto-envs", "--op", "cleanup"]
    }
  }
}
```

The reference runtime reports a direct `socket_path` endpoint. The
`socket_proxy` endpoint form is an authoring option for runtimes whose
sockets are only reachable inside the environment (see "Endpoints and
liveness"); its production behavior — bridge lifecycle, readiness probing,
EOF-pushed death — is exercised by the proxy scenarios in
`quecto-agentic-harness/tests/features/script_managed_liveness_slice3.feature`.
Forwarded nested descendants do not run the top-level endpoint negotiation in
their ancestor session; when such a descendant reports a script-managed direct
socket, ancestors omit `socketPath` and live commands return a clear
non-connectable-socket error instead of exposing the container-local path.

## The official Docker/Podman adapter

Alongside the host-local reference set (which remains the CI-exercised
default), the repository ships an official Docker/Podman adapter implementing
the same contract (the directory keeps its historical `docker` name):

- [`scripts/container-runtime/docker/create.sh`](../scripts/container-runtime/docker/create.sh)
- [`scripts/container-runtime/docker/exec.sh`](../scripts/container-runtime/docker/exec.sh)
- [`scripts/container-runtime/docker/inspect.sh`](../scripts/container-runtime/docker/inspect.sh)
- [`scripts/container-runtime/docker/kill.sh`](../scripts/container-runtime/docker/kill.sh)

Design properties:

- **Rootless Podman by default, Docker as fallback.** Every script resolves
  its CLI to `podman` when present, else `docker`; `QUECTO_CONTAINER_CLI`
  overrides. Rootless Podman is the intended runtime for an autonomous
  spawner: membership of the `docker` group is root-equivalent on the host
  (the daemon runs as root with no policy layer, so anything holding the
  socket can bind-mount `/` and escalate), whereas rootless Podman runs the
  container as the invoking user inside a user namespace. `create.sh` runs
  the child as the host uid/gid and, under Podman, adds `--userns=keep-id`
  so the identity-mounted paths keep their ownership inside the container.
  The `metadata.runtime` field reports whichever CLI was used.
- **A real init at PID 1.** `create.sh` passes `--init` so a minimal init
  (catatonit under Podman, tini under Docker) reaps orphaned grandchildren
  and forwards signals. Without it the child is PID 1 itself: nested agents
  whose parent exited accumulate as zombies, and `stop` hangs until the
  SIGKILL escalation because PID 1 ignores an unhandled SIGTERM. The child
  remains the container's liveness; the init exits when the child does.
- **One container per environment, child as PID 1.** `create.sh` starts the
  child as the container's main process, so Docker's view of the container is
  exactly the child's liveness; `exec.sh` joins later members with
  `docker exec` into the same container.
- **Identity bind-mounts.** The per-environment workspace (rw), the parent's
  socket dir (rw), the child binary (ro), the child's `--config` file (ro,
  when outside `$HOME/.quecto`), and `$HOME/.quecto` (rw) are mounted at the
  same path inside and outside, so the child's CLI args need no rewriting and
  the UDS socket it binds appears directly on the host.
- **`HOME` preserved, `QUECTO_BASE_DIR` never overridden.** `QUECTO_BASE_DIR`
  is quecto's credentials/config home; overriding it inside the container
  detaches the child from the identity-mounted `$HOME/.quecto` and breaks
  OAuth providers. The scripts carry a comment warning against this.
- **Image selection.** `--image <img>` on the create argv, or the
  `QUECTO_DOCKER_IMAGE` environment variable, with a sensible local default
  (`quecto-box:local`).
- **Pid fence.** `create.sh` passes `--pids-limit` (default `16384`;
  `QUECTO_CONTAINER_PIDS_LIMIT` overrides, `-1` defers to the user slice,
  `0` is refused). Threads count against the container's pid cgroup and the
  runtime default of 2048 is exhausted by an in-container `cargo test`,
  after which every fork fails and the environment dies with all its
  members.
- **Preflight before any state exists (#2024 S4b).** `create.sh` runs one
  list of checks — runtime CLI (`podman`/`docker`, `QUECTO_CONTAINER_CLI`),
  `jq`, `git` (when `--repo` is given), `gh` (a warning only: without it
  members get no GitHub token), the image present in the local store
  (`podman image exists` / `docker image inspect`; **never an implicit
  pull**), `--repo` reachable (`git ls-remote`, bounded by
  `QUECTO_REPO_CHECK_TIMEOUT`, default 15 s, no credential prompt), and the
  state dir writable and owned by the current user (or, when it does not
  exist yet, creatable under a writable parent) — before the environment
  directory is created. A normal create dies at the first
  failure with a message that names the remedy and a distinct exit code
  (2 usage, 3 no runtime, 4 no jq, 5 no git, 6 image missing, 7 `--repo`
  unreachable, 8 state dir); `--preflight-only` evaluates every check and
  prints the tab-separated report `quecto container doctor` presents,
  leaving no trace (not even the state dir). A runtime that cannot answer
  the image lookup (daemon down, socket permission, `QUECTO_REPO_CHECK_TIMEOUT`
  exceeded) is its own failure with exit 3, never "image missing"; an
  empty repository (no `HEAD`) is a warning, since the clone accepts it.
  The host-local reference `create.sh` implements the same mode with its
  own subset (jq, git, `--repo`, state dir).
- **Rollback and containment.** `create.sh` installs an ERR trap that removes
  partial state and `docker rm -f`s any container it managed to start; every
  destructive operation proves the environment id contains no path
  separators and resolves under the trusted `--state-dir` root, mirroring the
  host-local set. `kill.sh` serves `--op kill` / `--op cleanup` and logs each
  operation to the state root. Under Podman it bounds the remove's grace to
  one second (`--time 1`): Docker's `rm -f` kills immediately, Podman's would
  otherwise wait the container's ten-second stop timeout, which does not fit
  the parent's signal-driven exit budget.
- **Strict JSON contract.** All stdout results are emitted with `jq`, exactly
  matching the `create`/`exec`/`inspect` wire contracts above. `create.sh`
  reports `metadata.checkout` (the members' working directory, which is the
  swarm checkout root) so the supervising session can keep a swarm's
  environment when its coordinator is lost (#1924).
- **Member harness logs reach journald.** `create.sh` passes
  `-e RUST_LOG=${RUST_LOG:-info}`; without it the member harness's
  redacting subscriber is a no-op and an environment that dies leaves no
  trace of why (termination signal, teardown, socket close). Set `RUST_LOG`
  on the host before spawning to change the level for that environment. The
  variable reaches the member harness only: the harness scrubs it (with the
  `QUECTO_SWARM_*` identity) from every tool child it starts, so a member's
  `cargo test` or other Rust programs keep their own logging defaults.
  Under rootless Podman with its default `journald` log driver the
  container's stdout/stderr land in the user journal, so read a member's
  logs with:

  ```sh
  journalctl --user CONTAINER_NAME=quecto-env-<id>
  ```

  where `env-<id>` is the environment id from `get_containers` (the
  container is named `quecto-<environment_id>`). Add `-f` to follow, or
  `--since`/`--until` around the time an environment was lost. Under Docker
  (default `json-file` driver) read them with `docker logs quecto-env-<id>`
  instead; a Podman configured with another driver likewise uses
  `podman logs`. `kill.sh`
  additionally logs every kill/cleanup it performs to `kill.log` in the
  state root, which is the first place to look when an environment vanished.
- **Host-side clone is transport-restricted.** The repo URL from the config's
  own `--repo` argv is cloned on the host before any container exists, so
  `create.sh` runs `git clone` under `GIT_ALLOW_PROTOCOL=file:https:ssh:git` —
  command-running transports (`ext::…`) can never execute host commands
  (PR #1401 review; config files travel, so the restriction stays).
- **Provider API keys never enter the docker-side container config.**
  `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `OPENROUTER_API_KEY` /
  `FIREWORKS_API_KEY` are written to a `0600` file in the `0700` state dir,
  identity-mounted read-only, and sourced by a bootstrap `sh` that `exec`s
  the child (which therefore still ends up as PID 1). Passing them with
  `docker run -e` would persist them in the container config, readable via
  `docker inspect` for the container's whole lifetime (PR #1401 review).
  Requires `/bin/sh` in the image.
- **GitHub access works inside the environment.** Agent workflows need `gh`
  and git-over-https pushes, and a host keyring is unreachable from a
  container. `create.sh` resolves the token host-side (`gh auth token`) and
  ships it as `GH_TOKEN`/`GITHUB_TOKEN` through the same `0600` secret file;
  git identity (global gitconfig only, for determinism) and the
  `gh auth git-credential` helper travel as non-secret `GIT_CONFIG_*`
  environment entries, so the host gitconfig — which may carry LFS filters or
  keyring helpers the image lacks — is never mounted. `exec.sh` gives joiners
  the identical contract (including sourcing the secret file). Requires `gh`
  in the image for API/push use; everything else degrades gracefully when no
  token is available.

A matching configuration:

```json
{
  "container_configs": {
    "docker": {
        "default": true,
        "create": ["/repo/scripts/container-runtime/docker/create.sh", "--state-dir", "/var/tmp/quecto-docker-envs", "--repo", "https://github.com/you/project"],
        "exec": ["/repo/scripts/container-runtime/docker/exec.sh", "--state-dir", "/var/tmp/quecto-docker-envs"],
        "inspect": ["/repo/scripts/container-runtime/docker/inspect.sh", "--state-dir", "/var/tmp/quecto-docker-envs"],
        "kill": ["/repo/scripts/container-runtime/docker/kill.sh", "--state-dir", "/var/tmp/quecto-docker-envs", "--op", "kill"],
        "cleanup": ["/repo/scripts/container-runtime/docker/kill.sh", "--state-dir", "/var/tmp/quecto-docker-envs", "--op", "cleanup"]
    }
  }
}
```

CI has no container runtime, so the Docker/Podman adapter is not exercised by
the CI BDD lanes; it is verified manually against local rootless Podman, and its
shape (existence, fail-fast mode, contract needles, cross-links) is pinned
by `quecto-agentic-harness/tests/docs/container_runtime_docs.rs`.

## Troubleshooting (runbook)

Every container failure now carries the script's own words; the first
move is always the same: read the tool error (or the harness stderr),
then run the doctor from the directory the agent runs in and apply the
remedy it prints.

```
quecto container doctor                 # the effective default config of this directory
quecto container doctor --name <config> # a named entry
quecto container doctor --config <file> # that file's entries, ignoring the overlay
```

The doctor resolves the container config exactly as `spawn container:
true` does (global file plus the directory's trusted `.quecto/config.json`
overlay; an untrusted overlay that declares `container_configs` is
reported and withheld), runs the create script's `--preflight-only` mode
**without creating an environment**, prints one line per check
(`✓` passed, `!` warning, `✗` failed) with a `remedy:` line under each
warning or failure, and exits 1 when any check failed:

```
container config "quecto" (create: /…/docker/create.sh --state-dir /var/tmp/envs --repo https://github.com/you/project)
  ✓ runtime-cli  podman at /usr/bin/podman
  ✓ jq           jq at /usr/bin/jq
  ✓ git          git at /usr/bin/git
  ✓ gh           gh at /usr/bin/gh
  ✗ image        image quecto-box:local is not present in the local podman store
    remedy: build it (podman build -t quecto-box:local <dir with its Containerfile>) or pull it (podman pull quecto-box:local); create never pulls implicitly
  ✓ repo         --repo https://github.com/you/project is reachable
  ✓ state-dir    state dir /var/tmp/envs is writable and owned by the current user
1 check failed, 0 warnings
```

| Symptom (tool error / stderr) | Doctor line | Remedy |
| --- | --- | --- |
| `script-managed create failed with status exit status: 3: … no container runtime on PATH` | `✗ runtime-cli` | Install rootless Podman (preferred) or Docker and put it on PATH, or set `QUECTO_CONTAINER_CLI`. |
| `… status exit status: 4: … jq is not on PATH` | `✗ jq` | Install `jq`. |
| `… status exit status: 5: … git is not on PATH` | `✗ git` | Install `git` (only needed for configs with `--repo`). |
| `… gh is not on PATH: members will have no GitHub token` (a warning; the create proceeds) | `! gh` | Install `gh` and run `gh auth login` so members can push and use the GitHub API. |
| `… status exit status: 6: … image quecto-box:local is not present` | `✗ image` | Build the image (`podman build -t quecto-box:local <dir>`) or pull it; the scripts never pull implicitly. Change `--image` / `QUECTO_DOCKER_IMAGE` if another image was meant. |
| `… status exit status: 7: … --repo <url> is unreachable: fatal: …` | `✗ repo` | Check the URL and your credentials (`ssh` key or `gh auth login`); fix `--repo` in the config's create argv. |
| `… status exit status: 8: … state dir <dir> is not owned by the current user` / `cannot be created` | `✗ state-dir` | Point `--state-dir` at a directory you own. |
| `unknown container config '<name>' (available container configs: …)` | (doctor refuses too) | Pick a listed name, or bind the repository: `quecto config set --local container_configs.<name> '{…,"default":true}'`. |
| `container: true refused: the checkout's repo-local config overlay was not applied` | (doctor refuses too) | Review `.quecto/config.json`, then `quecto config trust` in that directory. |
| `… status exit status: 3: … podman could not look up image …` / `did not answer within 15s` | `✗ image` (runtime cause quoted) | The runtime is installed but cannot answer: start the daemon/service (`podman info`, `systemctl --user start podman.socket`, docker group membership). |
| `container config '<name>': create script … does not support --preflight-only` | — | The config's create script predates the preflight contract (both shipped script sets implement it); add the mode to a custom script (see "Script contract"). |
| `… status exit status: 1: …` after the preflight (clone, `podman run`) | all `✓` | Read the quoted stderr: the clone or the runtime refused. The create rolled its environment back; `kill.log` in the state dir lists every kill/cleanup. |
| `script-managed exec failed with status …: …` | — | The join's own stderr is quoted; the environment may have exited — `agent_cmd get_containers` shows its status. |

Eight create-then-rollback cycles in one afternoon with nothing but
`status 1` to show for them is what this runbook replaces (#2024).

## How to author another runtime adapter

To adapt the reference runtime to Docker, Podman, devcontainers, or any
other isolation mechanism, copy `scripts/container-runtime/` and replace
only the marked `--- Runtime-specific section ---` in each script; the argv
parsing, environment-variable handling, and JSON results stay identical.
Rules an author must keep:

1. **Structured argv, no shell interpolation.** Every configured operation
   is an argv array executed directly; treat your own `--repo` value and the
   child command after `--` as opaque literals.
2. **Start the child exactly once.** `create`/`exec` own the child's whole
   lifetime inside the environment; Quecto never starts a fallback child.
3. **Exactly one JSON object on stdout**, produced by a real JSON encoder,
   with exactly the documented fields (`environment_id`, `workspace_path`,
   `metadata`, and exactly one of `socket_path`/`socket_proxy` for `create`;
   `metadata` plus one endpoint for `exec`; `metadata` plus optional
   `status` for `inspect`). Logs go to stderr only.
4. **Honor the identity split.** `create` receives the session ref in
   `QUECTO_CONTAINER_ENVIRONMENT_REF`; every later operation receives the
   runtime-owned id you reported, in `QUECTO_CONTAINER_ENVIRONMENT_ID`.
5. **Trusted-root containment before destructive cleanup.** Never remove a
   path you have not proven to resolve under your own trusted state root.
6. **Keep runtime knowledge in the scripts.** Quecto's Rust code contains no
   Docker/Podman/devcontainer special cases and must never need any.

## Planned inference-admission transport (#1679)

[ADR-0026](../quecto-agentic-harness/docs/architecture-design-records/adr-0026-shared-inference-admission.md)
fixes the private admission transport contract. **Not implemented or enabled by
this documentation:** existing container scripts do not yet provide admission.
The planned official same-host Docker/Podman adapter uses a dedicated private
admission socket-directory mount; create, join and nested launches must verify
reachability to the same authority. Custom runtimes without path visibility need
an explicit child-to-host reverse bridge capability. Existing `socket_proxy`
connects the parent to the child and is not proof of this reverse capability.
Never expose the entire agent-control socket directory as an admission endpoint.

The P0 prototype in
`quecto-agentic-harness/tests/integration/inference_admission_transport.rs` uses real local
processes and a test-only stdio bridge, not Docker or production admission.
Actual supported-runtime create/join/nested, cancellation and restart evidence is
required in P3 before activation. Unsupported enabled transport must fail closed,
not silently substitute an in-process or container-local budget. No multi-host
coordination is promised.
