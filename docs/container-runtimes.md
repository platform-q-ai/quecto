# Container runtime scripts

Quecto container-backed spawn is script-managed behind the `AgentLaunchBackend` port. Core spawn builds the final child CLI first, then asks the selected backend to launch it. Local spawn remains the default. Script-managed runtimes are selected only when `container_scripts` are configured and a spawn request asks for a new or existing environment.

Configure `container_scripts.default` and named `container_scripts.scripts` entries with `create`, `exec`, `inspect`, and `kill` commands. `container: true` / `mode: new` uses the requested `container_script`/`containerScript` or the default; `mode: existing` uses the default script set. Missing, unknown, or incomplete script sets fail before launch.

Launch semantics:
- **new**: Quecto invokes `create ... -- <child-binary> <child-args...>`. The create script must start the child through the runtime mechanism and return endpoint JSON. Quecto does not follow with `exec`.
- **existing**: Quecto invokes `exec ... -- <child-binary> <child-args...>` for an already known runtime reference/name.

Scripts receive structured argv plus environment including `QUECTO_AGENT_UUID`, `QUECTO_PARENT_AGENT_UUID`, `QUECTO_ENVIRONMENT_UUID`, `QUECTO_CONTAINER_REF`, `QUECTO_REPO_URL`, `QUECTO_WORKSPACE_PATH`, and `QUECTO_CONTAINER_ROOT`. Human logs go to stderr. Stdout must be exactly one JSON object produced by a real JSON encoder.

Required JSON fields:
- `create`/`exec`: `environment_id`, `workspace_path`, `container_ref`, and either `socket_path` (direct UDS) or `socket_proxy` (runtime proxy endpoint), plus `metadata`.
- `inspect`: `environment_id`, `status`, `health`, `workspace_path`, `container_ref`, plus `metadata`.
- `kill`: `environment_id`, `status`, cleanup result, `workspace_path`, `container_ref`, plus `metadata`.

Quecto treats the returned endpoint as typed parent connectivity (`direct` UDS path or proxy). Startup readiness is bounded; after startup Quecto owns liveness by maintaining a persistent child connection and using EOF/reset as the death signal, then running one inspect and broadcasting the authoritative status.

The reference scripts in `quecto-agentic-harness/scripts/container-runtime/` are honest host-local examples of the contract. They are not Docker provisioning. Runtime-specific destruction belongs in scripts; those scripts must canonicalize roots/workspaces and prove containment before destructive cleanup. Quecto core also rejects unsafe repository input and never relies on shell-interpolated JSON.