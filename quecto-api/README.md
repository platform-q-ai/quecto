# quecto-api

HTTP/WebSocket gateway to a quecto agent over UDS.

Connects to a running `quecto agent --mode uds` process via Unix domain socket
and exposes its capabilities as a REST + WebSocket API for web applications.
Version **0.5.3**.

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/health` | Health check (`200` when the UDS agent is connected, `503` when not). Body: `{"healthy":bool,"agent_connected":bool}` |
| `POST` | `/prompt` | Send a prompt. Body: `{"message":"..."}` plus optional `streamingBehavior` (`"steer"` / `"followUp"`) and `waitForCompletion` (default `true`). With `waitForCompletion: false`, the gateway enqueues and returns `{"accepted":true}` without waiting for the run |
| `POST` | `/steer` | Interrupt after the current tool, then deliver a message. Body: `{"message":"..."}` (non-empty) |
| `POST` | `/follow_up` | Queue a message for when the current run finishes. Body: `{"message":"..."}` |
| `POST` | `/abort` | Cancel the current agent run (empty body) |
| `POST` | `/model` | Switch the active model. Body: `{"model":"provider/id"}` **or** both `{"provider":"...","modelId":"..."}`. Blank/whitespace fields and partial split targets are rejected with `400` |
| `POST` | `/effort` | Set session reasoning effort. Body: `{"effort":"..."}`. Accepted values (case/whitespace normalized): `none`, `low`, `medium`, `high`, `xhigh`, `max`. Unknown values → `400` |
| `POST` | `/clear_history` | Clear conversation history in-place without restarting the agent (empty body) |
| `GET` | `/subagents` | List spawned subagents and their live status (#524) |
| `GET` | `/tools` / `/tools/catalogue` | Return the agent rich tool catalogue (`get_tool_catalogue`) |
| `GET` | `/state` | Get current agent state (`get_state`) |
| `GET` | `/messages` | Newest bounded history page (#1061). Query: optional `?before=<messageId>` pages backward. Response `data` carries `before` / `hasMoreBefore` |
| `GET` | `/messages/tail?n=N` | Last N messages (defaults to 10). Maps to the harness `get_messages` count / tail path |
| `GET` | `/messages/{id}` | Resolve one message by stable id (#1060). Query: optional `agent_id`, `offset`, `limit` for child lookup and bounded content recovery (#1094) |
| `GET` | `/audit/events` | Read the session audit JSONL log. Query: optional `after` (line offset, default 0), `limit` (default 500, max 2000). Response: `{"data":{"events":[...],"next_offset":N}}`. Path: `$QUECTO_BASE_DIR/audit/<session>.jsonl` (`QUECTO_BASE_DIR` default `/home/appuser/.quecto`, `QUECTO_SESSION_KEY` default `default`) |
| `GET` | `/stats` | Session statistics (`get_session_stats`) |
| `WS` | `/ws` | WebSocket event stream (tokens, tool executions, turn boundaries, …). Clients may also send JSON prompts and direct `get_message` commands (see below) |

Successful control/query responses are the agent's UDS `response` (or correlated) event JSON. Failures map to:

| Status | When |
|--------|------|
| `400` | Invalid request (empty message, unknown effort, incomplete model target, …) |
| `409` | Agent busy without `streamingBehavior` (when the agent reports busy) |
| `503` | Agent not connected |
| `504` | Gateway timed out waiting for the correlated UDS response (120s) |
| `500` | Internal / unexpected gateway error |

CORS is permissive (`CorsLayer::permissive`) for browser clients.

## Usage

```bash
# Start the agent
quecto agent --mode uds --socket /tmp/quecto.sock --persist

# Start the API gateway (defaults: host 127.0.0.1, port 8080)
quecto-api --socket /tmp/quecto.sock --host 0.0.0.0 --port 8080
```

### CLI flags

| Flag | Env fallback | Default | Description |
|------|--------------|---------|-------------|
| `--socket <path>` | `QUECTO_SOCKET` | *(required)* | Path to the agent's Unix domain socket |
| `--host <addr>` | — | `127.0.0.1` | HTTP bind host |
| `--port <u16>` | — | `8080` | HTTP bind port |

Unknown flags and missing `--socket` / `QUECTO_SOCKET` exit with a non-zero status and a short error on stderr. The process handles `SIGINT` / `SIGTERM` for graceful shutdown.

### Prompt body

```json
{
  "message": "Summarize README.md",
  "streamingBehavior": "steer",
  "waitForCompletion": true
}
```

- `message` — required, non-empty.
- `streamingBehavior` — optional; required by the agent when a run is already in progress (`"steer"` or `"followUp"`).
- `waitForCompletion` — optional, default `true`. When `false`, the gateway uses fire-and-forget enqueue and returns acceptance immediately; follow the run on `/ws`.

### WebSocket (`/ws`)

On connect the gateway subscribes to the agent's broadcast event stream and forwards every event as a JSON text frame (except duplicate direct command responses already returned to the requester).

**Client → gateway text frames:**

1. **Prompt** (legacy shape): `{"message":"...","streamingBehavior":"...","waitForCompletion":...}` — empty messages are ignored. Enqueued as a UDS `prompt` (completion is observed via the event stream, not a correlated HTTP-style wait).
2. **Direct `get_message`** (#1094):  
   `{"type":"get_message","id":"...","messageId":"...","agent_id":"...","toolCallId":"...","offset":0,"limit":65536}`  
   Correlated `response` is written back on the same socket with the client `id` echoed. Malformed `get_message` frames yield `success: false` with `command: "get_message"`.
3. **Direct ledger `sync`** (#1195):  
   `{"type":"sync","id":"...","epoch":1,"sinceRev":42,"agent_id":"..."}`  
   Pulls committed ledger messages after `sinceRev` for `epoch`; omit `agent_id` for the root agent or set it to target a child. The correlated `response` echoes the client `id` and carries the agent's sync payload, for example `{"epoch":1,"rev":45,"messages":[...],"nextRev":45,"caughtUp":true}`. Resync metadata from stale/future cursors is forwarded in the response payload. Malformed `sync` frames yield `success: false` with `command: "sync"` and are not sent to the agent.

Oversized messages should be recovered by walking `offset` / `nextOffset` / `hasMoreContent` or ledger `sync` cursors rather than relying on a single frame.

### Event shapes (subset)

Gateway domain events mirror the UDS wire types the client cares about, including:

- `agent_end` — `messages` is legacy/empty after harness #1060; use `messageRefs` + `GET /messages/{id}` (or WS `get_message`) for content.
- `turn_end` — assistant turn payload + `toolResults`.
- `subagent_messages_appended` — child turn refs (`agent_id`, `messageRefs`).
- `response` — command ack (`id`, `command`, `success`, optional `data` / `error`). For `command: "sync"`, `data` contains the ledger cursor metadata and messages returned by the agent.
- `ledger_advanced` — `{"type":"ledger_advanced","epoch":1,"rev":45}`. Treat it as a hint that newer ledger entries exist; use `sync` with the last durable cursor to recover missed/skipped broadcast tail events.
- `token`, `tool_execution_start` / `tool_execution_end`, `agent_start`, `turn_start`.

Unknown future agent event types deserialize as `unknown` unless explicitly modeled; ledger-related future events modeled by the gateway preserve their flattened payload. Unparseable lines are logged and dropped.

## Architecture

```
src/
├── domain/           # ApiError, AgentEvent (no infra deps)
├── application/      # Use cases + ports (AgentGateway trait)
├── infrastructure/   # UdsGateway (quecto-line-io framed/legacy reader), axum HTTP router
└── interface/        # CLI Config parse + composition root (bind/serve)
```

- **Domain** and **application** layers have zero dependency on HTTP or UDS.
- The `AgentGateway` port (`send` / `enqueue` / `subscribe` / `is_connected`) is implemented by `UdsGateway` in the infrastructure layer.
- `UdsGateway` writes legacy NDJSON commands during the ADR-0008 deprecation window (interop with both agent generations) and reads dual-mode frames via `quecto-line-io` (8 MiB cap).
- Tests use an in-crate `MockGateway` — no real agent needed for unit/BDD of use cases and most routes.
- `main` is a thin shim: parse config → `interface::server::bind` → graceful `serve`.

### Enforced boundaries

The Clean Architecture boundaries are enforced by executable BDD scenarios in
`tests/features/architecture.feature`, which fail the build if:

- the `domain` layer imports from `application`, `infrastructure`, or `interface`;
- the `application` layer imports from `infrastructure` or `interface`; or
- the `application` layer references any concrete transport type
  (`axum`, `hyper`, `tokio::net`, `UnixStream`, `WebSocket`).

Every use case takes `&dyn AgentGateway` (a port), never a concrete gateway, so
control commands can be exercised with the in-crate `MockGateway` test double.

## Development

```bash
cargo test -p quecto-api      # Unit tests + BDD scenarios (tests/bdd)
cargo clippy -p quecto-api    # Lint
```

BDD feature files live under `tests/features/` (`health`, `prompt`, `commands`,
`state`, `websocket`, `architecture`). Label-triggered authoritative CI coverage for this crate is gated
at the same function-coverage bar as the other library crates (95%).
