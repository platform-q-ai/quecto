# Quecto

Quecto is a Rust workspace for running AI coding agents locally, connecting them to terminal, HTTP/WebSocket, MCP, and managed-runtime surfaces. The core `quecto` binary owns model sessions, credentials, tools, workflows, subagents, and the Unix-domain-socket (UDS) protocol; the companion crates are clients, bridges, and infrastructure around that kernel.

Companion crate versions are declared in each package `Cargo.toml`.

## Workspace projects

| Project | Package / binary | What it is | Main features |
|---|---|---|---|
| [Agentic harness](quecto-agentic-harness/) | `quecto-agentic-harness` / `quecto` | The core CLI and UDS agent kernel. | Model/provider configuration, credential management, tool execution, workflows, subagents, session history, audit logs, UDS protocol, context management, and local/container execution hooks. |
| [Terminal UI](quecto-tui/) | `quecto-tui` | A lightweight terminal client for the `quecto` UDS agent. | Spawn-or-attach agent startup, streaming chat, slash commands, model switching, workflow controls, markdown/link rendering, input history, and tool-output expansion. |
| [HTTP/WebSocket API](quecto-api/) | `quecto-api` | A REST and WebSocket gateway to a running UDS agent. | `/prompt`, `/steer`, `/follow_up`, `/abort`, `/model`, `/state`, `/messages`, `/stats`, `/tools`, `/audit/events`, and `/ws` for browser/server clients. |
| [MCP bridge](quecto-mcp/) | `quecto-mcp` | A standalone UDS extension that exposes remote MCP server tools as Quecto tools. | MCP tool discovery, allow/deny filtering, Quecto-safe tool-name mapping, `register_tools`, and proxying Quecto `execute_tool` events to MCP `tools/call`. |
| [Runtime manager](quecto-runtime-manager/) | `quecto-runtime-manager` | A small HTTP manager for per-session Quecto runtimes. | Ensure/stop/status runtime APIs, process and Kubernetes pod execution models, API proxying, credential secret patching, and runtime capacity limits. |
| [Line/framing I/O](quecto-line-io/) | `quecto-line-io` | Shared bounded JSON/UDS reader and writer library. | ADR-0008 length-prefixed frames, legacy NDJSON compatibility, 8 MiB payload cap, oversized-input handling, and shared wire helpers used by the harness, TUI, and API. |

Package-specific source, tests, docs, and fixtures are colocated under each package root.

## Values

Quecto is built around a few practical engineering values:

- **Local-first control:** developers should be able to run, inspect, and supervise agent sessions from their own machine and infrastructure.
- **One kernel, many surfaces:** the CLI/UDS harness is the source of truth; the TUI, API, MCP bridge, and runtime manager are focused adapters around it.
- **Explicit, recoverable protocols:** commands, events, history, and tool results should be observable, bounded, and recoverable rather than hidden in client state.
- **Security before convenience:** credentials stay out of the repo, secrets are redacted, local env files are ignored, and sandbox limits are documented instead of implied.
- **Small, verified changes:** contributors should prefer minimal changes, repository conventions, BDD/TDD where practical, and checks that prove the touched behavior.
- **Composable automation:** workflows, tools, subagents, and runtimes should be independently understandable pieces that compose without surprising authority or identity boundaries.

## Features at a glance

- **Local-first agent kernel:** run `quecto` directly or as a persistent UDS server.
- **Multiple user surfaces:** terminal UI, REST, WebSocket, and MCP-extension workflows all speak to the same agent kernel.
- **Provider-aware configuration:** configure built-in and OpenAI-compatible providers via config files, environment variables, or the credential store.
- **Workflow support:** opt into guided development workflows with optional guard checks.
- **Subagents:** spawn background agent sessions for independent investigation or review work.
- **Session and message recovery:** stable message IDs, paged history, bounded content recovery, and audit-event access.
- **Security-conscious defaults:** secret redaction in logs/status surfaces, explicit credential handling, ignored local env files, and documented sandbox limitations.

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
