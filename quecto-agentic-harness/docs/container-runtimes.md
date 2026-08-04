# Container runtime scripts

Quecto container-backed spawn is script-managed. Configure `container_scripts.default` and named `container_scripts.scripts` entries with `create`, `exec`, `inspect`, and `kill` commands. `container: true` or `mode: new` uses the requested `container_script`/`containerScript` or the configured default; missing, unknown, or incomplete script sets fail before create.

Each command receives Quecto context through argv and environment such as `QUECTO_AGENT_UUID`, `QUECTO_PARENT_AGENT_UUID`, `QUECTO_ENVIRONMENT_UUID`, `QUECTO_CONTAINER_REF`, `QUECTO_REPO_URL`, and `QUECTO_WORKSPACE_PATH`. Human logs go to stderr. Stdout must be exactly one JSON object.

Required JSON fields:
- `create`: `environment_id`, `workspace_path`, `container_ref`, and `socket_path` or `socket_proxy`, plus `metadata`.
- `exec`: `environment_id`, `workspace_path`, `container_ref`, and `socket_path` or `socket_proxy`, plus `metadata`.
- `inspect`: `environment_id`, `status`, `health`, `workspace_path`, `container_ref`, plus `metadata`.
- `kill`: `environment_id`, `status`, cleanup result, `workspace_path`, `container_ref`, plus `metadata`.

Quecto owns exit detection by holding a liveness connection to the child UDS socket. Scripts do not poll for death; after EOF Quecto may call `inspect` once for post-mortem metadata.

The reference scripts in `scripts/container-runtime/` are a Docker-oriented starting template. Docker details belong in scripts, not in Quecto production code.
