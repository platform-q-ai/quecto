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
| `{"mode": "new", "repo"?: "...", "container_script"?: "..."}` | New environment; optional explicit repository URL and script-set name |

Unknown fields are rejected. Runtime-specific fields (`branch`, `pr`,
`image`, ...) do not exist. `{"mode": "existing"}` and proxy endpoints are
not yet supported and fail with a clear error.

- `repo` omitted → the parent checkout's `remote.origin.url` is used and
  must exist; explicit `repo` is passed to the script literally.
- Container spawns require the spawn call to pass `config` with an
  absolute path so `container_scripts` can be loaded from a trusted file.

A successful spawn returns a session-scoped environment reference
(`environment_ref=C1`, `C2`, ...). Refs are minted once per session and
never reused. The child then behaves like any other subagent: drive it
with normal `agent_cmd` operations over its direct UDS endpoint.

## Configuration

```json
{
  "container_scripts": {
    "default": "devcontainer",
    "scripts": {
      "devcontainer": {
        "create": ["/abs/path/to/create-script"],
        "cleanup": ["/abs/path/to/cleanup-script"]
      }
    }
  }
}
```

`default` names the script set used for `container: true`. Each script
set's `create`/`cleanup` are argv arrays executed directly — no shell
interpolation. Missing, unknown, empty, or unsafe (empty/NUL argument)
configuration fails before any script runs.

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
| `QUECTO_BASE_DIR` | Parent agent's base directory |

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

### `cleanup`

Invoked with `QUECTO_CONTAINER_ENVIRONMENT_REF` set to the
`environment_id` being destroyed. Runs exactly once when a launch fails
after creation (readiness, registration, or initial-prompt failure) or
when the environment is torn down.
