# Quecto Workspace

This repository is a Cargo workspace containing the Quecto agentic harness and related packages.

Current version: **0.99.1** (harness / `quecto` binary). Companion crate versions are declared in each package `Cargo.toml`.

## Packages

| Package | Path | Description |
|---|---|---|
| `quecto-agentic-harness` | [`quecto-agentic-harness/`](quecto-agentic-harness/) | Agentic harness; ships the `quecto` lib/bin (CLI/UDS agent) |
| `quecto-tui` | [`quecto-tui/`](quecto-tui/) | Terminal UI client for a UDS agent |
| `quecto-api` | [`quecto-api/`](quecto-api/) | HTTP/WebSocket gateway to a running UDS agent (v0.5.0) |
| `quecto-mcp` | [`quecto-mcp/`](quecto-mcp/) | MCP extension bridge (registers remote MCP tools over UDS) |
| `quecto-runtime-manager` | [`quecto-runtime-manager/`](quecto-runtime-manager/) | Runtime manager for provisioning/supervising isolated runtimes |
| `quecto-line-io` | [`quecto-line-io/`](quecto-line-io/) | Shared bounded framed-JSON / legacy-line UDS reader (used by harness, TUI, API) |

Package-specific source, tests, docs, and fixtures are colocated under each package root.

### Quick links

- Harness user guide and full CLI/UDS reference: [`quecto-agentic-harness/README.md`](quecto-agentic-harness/README.md)
- UDS wire protocol: [`quecto-agentic-harness/docs/uds-protocol.md`](quecto-agentic-harness/docs/uds-protocol.md)
- HTTP/WebSocket gateway: [`quecto-api/README.md`](quecto-api/README.md)
- Terminal UI: [`quecto-tui/README.md`](quecto-tui/README.md)
- MCP bridge: [`quecto-mcp/README.md`](quecto-mcp/README.md)
