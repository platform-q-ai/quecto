# Quecto

Quecto aims to be the smallest, most efficient agentic harness; designed to run anywhere on any device for any length of time.

## Features at a glance
- **Agentic kernel:** run `quecto` directly or as a persistent UDSSession and message recovery server.
- **Master Agents:** (do we want to keep this terminology?)
- **Subagents:** spawn background agent sessions for independent investigation or review work.
- **Providers:**
- **Workspaces:**
- **Containers:** run within containers
- **Workflows:**
- **Token-saving conversation management:** (talk about how conversation management works)
- **Keyboard shortcuts (planned):** config driven shortcut customisation, profile driven (default, vim, browser)

## Principles
- **Modular:** Quecto is extendable via .... 
- **Micro-Service driven:** Terminal UI, MCP bridge, Runtime manager and the Agentic harness all speak via UDS.

## Workspace projects

| Project | Package / binary | What it is |
|---|---|---|
| [Agentic harness](quecto-agentic-harness/) | `quecto-agentic-harness` / `quecto` | The core CLI and UDS agent kernel. |
| [Terminal UI](quecto-tui/) | `quecto-tui` | A lightweight terminal client for the `quecto` UDS agent. |
| [HTTP/WebSocket API](quecto-api/) | `quecto-api` | A REST and WebSocket gateway to a running UDS agent. |
| [MCP bridge](quecto-mcp/) | `quecto-mcp` | A standalone UDS-driven extension that exposes remote MCP server tools as Quecto tools. |
| [Runtime manager](quecto-runtime-manager/) | `quecto-runtime-manager` | A small HTTP manager for per-session Quecto runtimes. |
| [Line/framing I/O](quecto-line-io/) | `quecto-line-io` | Shared bounded JSON/UDS reader and writer library. |

Package-specific source, tests, docs, and fixtures are colocated under each package root.

## Quick start

### Prerequisites

- Rust stable with Cargo. The workspace uses Rust edition 2024.
- A model provider credential, usually via `quecto auth login` or provider environment variables.
- Unix-like environment for the UDS-based agent/client flows.

### Build

```bash
cargo build --workspace
```

### Install the main binaries locally

```bash
cargo install --path quecto-agentic-harness
cargo install --path quecto-tui
cargo install --path quecto-api
cargo install --path quecto-mcp
cargo install --path quecto-runtime-manager
```

### Configure credentials

Use the harness credential command for provider tokens:

```bash
quecto auth login --provider openai --token sk-proj-your-key
# or
quecto auth login --provider anthropic --token sk-ant-your-key
```

You can also use documented environment variables such as `OPENAI_API_KEY` and `ANTHROPIC_API_KEY`. See [the harness README](quecto-agentic-harness/README.md#configuration) for the full configuration model.

### Run the terminal UI

If `quecto` is on `PATH`, the TUI can spawn the kernel automatically:

```bash
quecto-tui
```

Workflow-driven launch:

```bash
quecto-tui --workflow --workflow-guards
```

### Run the agent and API gateway explicitly

```bash
# Terminal 1: start the core agent as a persistent UDS server
quecto agent --mode uds --socket /tmp/quecto.sock --persist

# Terminal 2: expose it over HTTP/WebSocket
quecto-api --socket /tmp/quecto.sock --host 127.0.0.1 --port 8080
```

Then send a prompt:

```bash
curl -s http://127.0.0.1:8080/prompt \
  -H 'content-type: application/json' \
  -d '{"message":"Summarize this repository"}'
```

### Register MCP tools

```bash
quecto-mcp \
  --socket /tmp/quecto.sock \
  --mcp-url https://perme8.example.com/mcp \
  --mcp-token "$PERME8_MCP_TOKEN" \
  --tool-prefix community.
```

## Configuration and local secrets

- Keep personal configuration and credentials out of the repository.
- `.env`, `.env.*`, private keys, cert/key bundles, logs, temp files, and build output are ignored by the repository and Docker build contexts.
- Prefer `quecto auth login`, provider environment variables, or deployment secret stores over committed config values.
- Treat example tokens in documentation/tests as placeholders only; never paste real credentials into tests, docs, issues, logs, or screenshots.

## Development

Install the repository hooks before making changes:

```bash
scripts/install-hooks.sh
source scripts/activate-hooks.sh
```

Common checks:

```bash
cargo fmt --all -- --check
cargo test --workspace --lib --bins
cargo clippy --workspace --all-targets --features quecto-agentic-harness/test-support -- -D warnings
```

Some crates also have BDD test targets and package-specific quality scripts. See [CONTRIBUTIONS.md](CONTRIBUTIONS.md) and the package READMEs for details.

## Documentation links

- Harness user guide and CLI/UDS reference: [quecto-agentic-harness/README.md](quecto-agentic-harness/README.md)
- UDS wire protocol: [quecto-agentic-harness/docs/uds-protocol.md](quecto-agentic-harness/docs/uds-protocol.md)
- HTTP/WebSocket gateway: [quecto-api/README.md](quecto-api/README.md)
- Terminal UI: [quecto-tui/README.md](quecto-tui/README.md)
- MCP bridge: [quecto-mcp/README.md](quecto-mcp/README.md)
- Runtime manager: [quecto-runtime-manager/README.md](quecto-runtime-manager/README.md)
- Shared line/framing I/O: [quecto-line-io/README.md](quecto-line-io/README.md)
- Docker harness for local TUI development: [docs/docker-harness.md](docs/docker-harness.md)
- Container runtimes for subagents: [docs/container-runtimes.md](docs/container-runtimes.md) (canonical script set: [scripts/container-runtime/](scripts/container-runtime/))

## Contributing, security, and license

- Contributing guide: [CONTRIBUTIONS.md](CONTRIBUTIONS.md)
- Security policy: [SECURITY.md](SECURITY.md)
- License: [LICENSE](LICENSE)
