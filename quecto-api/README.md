# quecto-api

HTTP/WebSocket gateway to a quecto agent over UDS.

Connects to a running `quecto agent --mode uds` process via Unix domain socket and exposes its capabilities as a REST + WebSocket API for web applications.

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/health` | Health check (200 connected, 503 disconnected) |
| `POST` | `/prompt` | Send a prompt to the agent |
| `POST` | `/steer` | Interrupt after the current tool, then deliver a message |
| `POST` | `/follow_up` | Queue a message for when the current run finishes |
| `POST` | `/abort` | Cancel the current agent run |
| `POST` | `/model` | Switch the active model (`{"model":"provider/id"}` or `{"provider":...,"modelId":...}`) |
| `GET` | `/subagents` | List spawned subagents and their live status (#524) |
| `GET` | `/extensions` | List registered extensions |
| `POST` | `/extensions/reload` | Re-scan and reload script extensions |
| `GET` | `/state` | Get current agent state |
| `GET` | `/messages` | Get the newest history page; `?before=<messageId>` pages backward. Response `data` carries `before`/`hasMoreBefore` cursors (paged per #1061) |
| `GET` | `/messages/tail?n=N` | Get last N messages |
| `GET` | `/stats` | Get session statistics |
| `WS` | `/ws` | WebSocket event stream (tokens, tool executions) |

## Usage

```bash
# Start the agent
quecto agent --mode uds --socket /tmp/quecto.sock --persist

# Start the API gateway
quecto-api --socket /tmp/quecto.sock --host 0.0.0.0 --port 8080
```

## Architecture

```
src/
├── domain/           # Error types, AgentEvent (no infra deps)
├── application/      # Use cases + ports (AgentGateway trait)
├── infrastructure/   # UDS client, axum HTTP router
└── interface/        # CLI entry point
```

- **Domain** and **application** layers have zero dependency on HTTP or UDS
- The `AgentGateway` port is implemented by `UdsGateway` in the infrastructure layer
- Tests use a mock gateway — no real agent needed

### Enforced boundaries

The Clean Architecture boundaries are enforced by executable BDD scenarios in
`tests/features/architecture.feature`, which fail the build if:

- the `domain` layer imports from `application` or `infrastructure`;
- the `application` layer imports from `infrastructure`; or
- the `application` layer references any concrete transport type
  (`axum`, `hyper`, `tokio::net`, `UnixStream`, `WebSocket`).

Every use case takes `&dyn AgentGateway` (a port), never a concrete gateway, so
control commands can be exercised with the in-crate `MockGateway` test double.

## Development

```bash
cargo test -p quecto-api      # Unit tests + BDD scenarios
cargo clippy -p quecto-api    # Lint
```
