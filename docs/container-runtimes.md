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
- `container_configs` load from a trusted config file: an explicit `config`
  argument in the spawn call wins; when it is omitted, the spawn falls back
  to the parent's own effective config path, so container spawns normally
  need no `config` argument. Whichever path applies must be absolute; when
  neither source exists the spawn fails with a clear error.

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
  status `running`, `empty` (live, no members), `killing`, `stopped`, or
  `cleanup-failed` (with its `last_error`), plus workspace and members.
- `kill_container` with `ref` or `name` — terminates every member agent,
  runs the environment's retained `kill` argv exactly once, and commits
  `stopped` only after the script succeeds. Its JSON result includes the
  environment ref and up to 20 terminated member agent ids/names, with
  `omitted_agents` when capped. A failed kill persists a retryable
  `cleanup-failed` state; run `kill_container` again to retry.

When the final member of a live environment exits or is killed, the same
retained `kill` operation runs exactly once (concurrent final exits cannot
double-kill). Script sets without a configured `kill` fall back to the
retained `cleanup` argv for final-member teardown; `kill_container` itself
refuses such environments up front, leaving every member untouched.

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
offer the menu. The spawn tool's description also carries the roster
(`Available container configs: ...`) as a session-start snapshot.

The container config in effect when an environment is **created** is
retained with the environment: later joins, kills, and inspects use the
retained `exec`/`kill`/`inspect` argv even if the labeled default changes
afterwards.

Repo-local container config overlays may be declared in
`<checkout>/.quecto/config.json` using the same `container_configs` shape.
Because that file is repository-controlled, Quecto gates it centrally before
any argv from it can be selected or executed. Trust is keyed by the
canonicalized (or absolute fallback) file path plus the raw file SHA-256, and
approval records are stored outside the repository under the user's Quecto
state/home data. A changed file hash requires a fresh approval. Until trusted,
repo-local entries are ignored visibly and global container configs remain
usable; once trusted, repo-local entries extend the global set, same-name
entries shadow global ones, and a repo-local default becomes the effective
default. The spawn tool roster is a session-start snapshot: if a repo-local
file appears or is approved later, spawn-time config loading still checks the
current trust/config state, but the already-rendered roster text is not live
reloaded. Container repository and auth semantics remain self-contained in the
selected config argv; the parent's cwd or checkout never supplies them.

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
lifecycle polling, no wrapper process. On death, the environment's
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
or an invalid contract fails the launch and rolls back.

The create result must contain exactly these fields — unknown keys are
rejected. When the config cloned a repository, the script should report it
as `metadata.repository`: that is how `get_containers` listings and the
TUI learn the source truthfully (sandbox configs report none and list an
empty repository).

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
`environment_id`. Runs exactly once per successful stop — either from
`kill_container` or when the final member exits. A non-zero exit leaves
the environment in a retryable `cleanup-failed` state with the script's
stderr preserved as the last error.

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

## The official Docker adapter

Alongside the host-local reference set (which remains the CI-exercised
default), the repository ships an official Docker adapter implementing the
same contract:

- [`scripts/container-runtime/docker/create.sh`](../scripts/container-runtime/docker/create.sh)
- [`scripts/container-runtime/docker/exec.sh`](../scripts/container-runtime/docker/exec.sh)
- [`scripts/container-runtime/docker/inspect.sh`](../scripts/container-runtime/docker/inspect.sh)
- [`scripts/container-runtime/docker/kill.sh`](../scripts/container-runtime/docker/kill.sh)

Design properties:

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
- **Rollback and containment.** `create.sh` installs an ERR trap that removes
  partial state and `docker rm -f`s any container it managed to start; every
  destructive operation proves the environment id contains no path
  separators and resolves under the trusted `--state-dir` root, mirroring the
  host-local set. `kill.sh` serves `--op kill` / `--op cleanup` and logs each
  operation to the state root.
- **Strict JSON contract.** All stdout results are emitted with `jq`, exactly
  matching the `create`/`exec`/`inspect` wire contracts above.
- **Host-side clone is transport-restricted.** The repo URL from the config's
  own `--repo` argv is cloned on the host before any container exists, so
  `create.sh` runs `git clone` under `GIT_ALLOW_PROTOCOL=file:https:ssh:git` —
  command-running transports (`ext::…`) can never execute host commands
  (PR #1401 review; config files travel, so the restriction stays).
- **Provider API keys never enter the docker-side container config.**
  `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `OPENROUTER_API_KEY` are written to
  a `0600` file in the `0700` state dir, identity-mounted read-only, and
  sourced by a bootstrap `sh` that `exec`s the child (which therefore still
  ends up as PID 1). Passing them with `docker run -e` would persist them in
  the container config, readable via `docker inspect` for the container's
  whole lifetime (PR #1401 review). Requires `/bin/sh` in the image.
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

CI has no Docker daemon, so the Docker adapter is not exercised by the CI
BDD lanes; it is verified manually against a local Docker daemon, and its
shape (existence, fail-fast mode, contract needles, cross-links) is pinned
by `quecto-agentic-harness/tests/container_runtime_docs.rs`.

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
