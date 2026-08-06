# Script-managed subagent environments (`container_scripts`)

Quecto can spawn a subagent into an isolated, script-managed environment
(e.g. a container) instead of a local child process. Quecto itself is
runtime-agnostic: it invokes an executable you configure, and that
executable owns Docker/Podman/devcontainer/whatever details. This is the
single canonical user document for the feature; the single canonical
reference script is [`scripts/container-script-reference.sh`](../scripts/container-script-reference.sh).

## Spawning

The `spawn` tool's `container` field selects the launch adapter:

| `container` value | Behavior |
| --- | --- |
| omitted or `false` | Local child process (default, unchanged) |
| `true` | New environment via the `container_scripts.default` script set |
| `{"mode": "new", "repo"?: "...", "container_script"?: "...", "name"?: "..."}` | New environment; optional explicit repository URL, script-set name, and environment name |
| `{"mode": "existing", "ref": "C1"}` | Join the existing session environment `C1` via its retained `exec` script |
| `{"mode": "existing", "name": "review-env"}` | Join an existing environment by its (unambiguous) name |

Unknown fields are rejected. Runtime-specific fields (`branch`, `pr`,
`image`, ...) do not exist. `mode: existing` requires exactly one of
`ref`/`name`; unknown, ambiguous, stopped, or stale (kill pending/failed)
targets fail without guessing, and proxy endpoints are still rejected.

- `repo` omitted → the parent checkout's `remote.origin.url` is used and
  must exist; explicit `repo` is passed to the script literally.
- Container spawns require the spawn call to pass `config` with an
  absolute path so `container_scripts` can be loaded from a trusted file.

A successful spawn returns a session-scoped environment reference
(`environment_ref=C1`, `C2`, ...). Refs are minted once per session and
never reused — a stopped environment stays listed and its ref is retired.
The child then behaves like any other subagent: drive it with normal
`agent_cmd` operations over its direct UDS endpoint. Every member of an
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
  `stopped` only after the script succeeds. A failed kill persists a
  retryable `cleanup-failed` state; run `kill_container` again to retry.

When the final member of a live environment exits or is killed, the same
retained `kill` operation runs exactly once (concurrent final exits cannot
double-kill). Script sets without a configured `kill` fall back to the
retained `cleanup` argv for final-member teardown; `kill_container` itself
refuses such environments up front, leaving every member untouched.

## Configuration

```json
{
  "container_scripts": {
    "default": "devcontainer",
    "scripts": {
      "devcontainer": {
        "create": ["/abs/path/to/create-script"],
        "cleanup": ["/abs/path/to/cleanup-script"],
        "exec": ["/abs/path/to/exec-script"],
        "kill": ["/abs/path/to/kill-script"]
      }
    }
  }
}
```

`default` names the script set used for `container: true`. Each script
set's operations are argv arrays executed directly — no shell
interpolation. `create` and `cleanup` are required; `exec` (joining) and
`kill` (explicit stop) are optional but needed for `mode: existing` and
`kill_container`. Missing, unknown, empty required, or unsafe (empty/NUL
argument) configuration fails before any script runs.

The script set in effect when an environment is **created** is retained
with the environment: later joins and kills use the retained `exec`/`kill`
argv even if `container_scripts.default` changes afterwards.

## Script contract

### `create`

Invoked as:

```
create-argv... -- <child-binary> <child-args...>
```

Environment variables provided to the script:

| Variable | Meaning |
| --- | --- |
| `QUECTO_CONTAINER_REPO` | Repository URL to check out (explicit or discovered) |
| `QUECTO_CONTAINER_SCRIPT` | Name of the selected script set |
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

All fields are required; `socket_path` must be a direct UDS endpoint the
parent can connect to (`socket_proxy` is rejected in this slice). Extra
JSON data after the object is rejected. A non-zero exit or an invalid
contract fails the launch and rolls back.

The create result must contain exactly these fields — unknown keys are
rejected.

### `exec`

Invoked to add another agent to an existing environment:

```
exec-argv... -- <child-binary> <child-args...>
```

Environment variables: `QUECTO_CONTAINER_SCRIPT` (the retained script-set
name) and `QUECTO_CONTAINER_ENVIRONMENT_ID` (the runtime `environment_id`
reported by `create` — not the session `C` ref). The script must start the
child inside the existing environment exactly once and print exactly one
JSON object:

```json
{
  "metadata": {},
  "socket_path": "/path/to/joined-child.sock"
}
```

`socket_path` must be a direct UDS endpoint; `socket_proxy` and unknown
keys are rejected.

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

### `cleanup`

Invoked with `QUECTO_CONTAINER_ENVIRONMENT_ID` set to the runtime
`environment_id` being destroyed (note: a different identity than the
session `C1` ref the create script received). Runs exactly once when a
launch fails after creation (readiness, registration, or initial-prompt
failure) — even when a `kill` is configured. For script sets without a
configured `kill`, the retained `cleanup` argv also serves as the
final-member teardown fallback.
