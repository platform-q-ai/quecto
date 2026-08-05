# Container runtime scripts

Quecto container-backed spawn is script-managed behind the `AgentLaunchBackend` port. Core spawn builds the final child CLI first, then asks the selected backend to launch it. Local spawn remains the default. Script-managed runtimes are selected only when `container_scripts` are configured and a spawn request asks for a new or existing environment. The design is runtime-agnostic: Quecto core never provisions a Docker image or assumes any specific container runtime.

Configure `container_scripts.default` and named `container_scripts.scripts` entries with `create`, `exec`, `inspect`, and `kill` commands. `container: true` / `mode: new` uses the requested `container_script`/`containerScript` or the default; `mode: existing` uses the default script set to join an existing environment. Missing, unknown, or incomplete script sets fail before launch.

Launch semantics:
- **new**: Quecto invokes `create ... -- <child-binary> <child-args...>`. The create script must create the runtime environment, start the child through that runtime mechanism exactly once, and return endpoint JSON. Quecto does not follow with `exec` and does not locally spawn a fallback child.
- **existing**: Quecto invokes `exec ... -- <child-binary> <child-args...>` for an already known runtime reference/name. The exec script starts the child exactly once in the existing environment; Quecto only records membership and endpoint/liveness metadata.

Scripts receive structured argv plus environment including `QUECTO_AGENT_UUID`, `QUECTO_PARENT_AGENT_UUID`, `QUECTO_ENVIRONMENT_UUID`, `QUECTO_CONTAINER_REF`, `QUECTO_REPO_URL`, `QUECTO_WORKSPACE_PATH`, and `QUECTO_CONTAINER_ROOT`. Human logs go to stderr. Stdout must be exactly one JSON object produced by a real JSON encoder.

Required JSON fields:
- `create`/`exec`: `environment_id`, `workspace_path`, `container_ref`, and either `socket_path` (direct UDS) or `socket_proxy` (runtime proxy endpoint), plus `metadata`.
- `inspect`: `environment_id`, `status`, `health`, `workspace_path`, `container_ref`, plus `metadata`.
- `kill`: `environment_id`, `status`, cleanup result, `workspace_path`, `container_ref`, plus `metadata`.

Quecto treats the returned endpoint as typed parent connectivity (`direct` UDS path or proxy). Startup readiness is bounded; after startup Quecto owns liveness by maintaining a persistent child connection and using EOF/reset as the death signal, then running one inspect and broadcasting the authoritative status. Inspect failures are persisted as `inspect_failed` health with `last_error` so await/status/TUI callers see a truthful retryable state instead of silent success.

Cleanup semantics are transactional. During spawn, a new environment is guarded from successful create through final registration and monitor readiness; if a later step fails, Quecto removes registry/subagent/member state, stops/reaps any Quecto-owned process, and invokes the environment kill command once when no live members remain. `kill_container` commits `stopped` only after member termination and kill-script cleanup succeed. Cleanup failures are persisted as `cleanup_failed`/`last_error` and the command returns an actionable retry error; membership and metadata remain available so another cleanup attempt can be made.

The reference scripts in `quecto-agentic-harness/scripts/container-runtime/` are honest host-local examples of the runtime-agnostic boundary contract. They are not Docker provisioning. Runtime-specific destruction belongs in scripts; those scripts must canonicalize roots/workspaces and prove containment before destructive cleanup. Quecto core also rejects unsafe repository input and never relies on shell-interpolated JSON.