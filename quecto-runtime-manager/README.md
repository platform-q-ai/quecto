# Quecto runtime manager

The runtime manager exposes a small HTTP API that starts, proxies, inspects, and stops per-session Quecto runtimes. It supports the current `process` and `pod` execution models only; it does not persist registry state across manager restarts.

## Configuration

Environment variables used by `src/main.rs`:

- `RUNTIME_MANAGER_HOST` / `RUNTIME_MANAGER_PORT` (default `0.0.0.0:8080`)
- `RUNTIME_MANAGER_TOKEN`: optional shared token. When set, mutating/proxy routes require `Authorization: Bearer <token>` or `x-quecto-token: <token>`.
- `QUECTO_RUNTIME_ROOT`, `QUECTO_SOCKET_ROOT`, `QUECTO_API_PORT_BASE`, `QUECTO_API_PORT_SPAN`, `QUECTO_MAX_RUNTIMES`
- `QUECTO_SYSTEM_PROMPT_PATH`, `QUECTO_CONFIG_PATH`, `QUECTO_CREDENTIALS_PATH`
- `MCP_URL`, `MCP_ALLOWLIST`, `MCP_TOKEN_PATH`
- `KUBERNETES_NAMESPACE`, `QUECTO_RUNTIME_POD_IMAGE`, `QUECTO_RUNTIME_POD_PULL_SECRET`
- `QUECTO_CREDENTIALS_SECRET_NAME`, `RUNTIME_MANAGER_SELF_URL`

## API

- `GET /health`: unauthenticated health and active runtime count.
- `POST /runtimes/ensure`: authenticated when `RUNTIME_MANAGER_TOKEN` is set. Body is `EnsureRuntimeRequest`; required fields are `agent_profile_id`, `project_id`, `chat_id`, `session_name`, and `session_key`. `execution_model` is omitted/`process` or `pod`. The runtime ref is deterministic from agent profile, project, and chat. First create returns `201`; repeated or concurrent ensures for the same ref return the existing runtime with `200` and do not start a second runtime.
- `DELETE /runtimes/{runtime_ref}`: authenticated when configured. Idempotent; returns `stopped: true` for an existing runtime and `false` after it is already gone. Pod runtimes delete their registered Kubernetes pod on explicit stop.
- `GET /runtimes/{runtime_ref}/status`: currently unauthenticated. Returns Kubernetes pod status for pod runtimes; process runtimes and unknown refs return `404`.
- `PUT /credentials`: authenticated when configured. Accepts `{ "credentials_json": "...valid JSON string..." }` and patches `credentials.json` in the configured Kubernetes Secret. Missing, non-string, or invalid inner JSON returns `400` without storing credentials; malformed outer JSON is rejected by Axum before route handling.
- `/runtimes/{runtime_ref}/ws` and `/runtimes/{runtime_ref}/*path`: authenticated proxy to the managed runtime API.

## Execution models and limits

Process runtimes spawn local `quecto agent`, `quecto-api`, and optional `quecto-mcp`, using a Unix socket under `QUECTO_SOCKET_ROOT`; overly long socket paths are rejected. Pod runtimes create a deterministic Pod manifest for the current image/config, seed credentials into a writable volume, run `quecto agent` plus `quecto-api`, and wire credential sync back to this manager. Capacity pressure reaps the oldest registry entry before starting a new runtime; explicit stop deletes registered pod resources.

Kubernetes operations require in-cluster service account files and API access in production. Unit tests use lifecycle seams/fakes for route coverage and do not require live Kubernetes, external HTTP, or Quecto binaries.
