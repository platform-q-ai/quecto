# quecto-api

HTTP/WebSocket gateway to a quecto agent over UDS.

Connects to a running `quecto agent --mode uds` process via Unix domain socket and exposes its capabilities as a REST + WebSocket API for web applications.

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/health` | Health check (200 connected, 503 disconnected) |
| `POST` | `/prompt` | Send a prompt to the agent |
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

## Development

```bash
cargo test -p quecto-api      # Unit tests + BDD scenarios
cargo clippy -p quecto-api    # Lint
```
