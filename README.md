# Quecto

Quecto is a small, efficient agentic harness for long-running software work. It is built around a simple belief: agents should be cheap to supervise, easy to interrupt, and able to delegate without forcing every integration to embed a heavyweight runtime.

The core `quecto` process can run as a one-shot CLI, an interactive REPL, or a persistent Unix-domain-socket (UDS) event bus. Companion clients and extensions talk to that socket using bounded JSON frames, so the terminal UI, HTTP/WebSocket gateway, MCP bridge, runtime manager, and subagents stay loosely coupled while sharing the same conversation state, tools, workflow engine, and recovery model.

## Features at a glance

- **Agentic kernel:** run `quecto` directly, in the REPL, or as a persistent UDS message and event server.
- **Subagents:** spawn background UDS-mode agents for independent implementation, investigation, and adversarial review while the parent stays responsive.
- **Provider support:** use OpenAI, Anthropic, ChatGPT Codex, or OpenAI-compatible endpoints through the same provider abstraction and credential store.
- **Workspace-aware tools:** give agents shell, file editing, search, docs, recall, workflow, and extension tools rooted in the active workspace and governed by repository hooks.
- **Container-capable delegation:** run subagents in configured container runtimes when isolation or reproducible environments matter.
- **Built-in workflows:** drive feature, refactor, chore, and adversarial-review loops with explicit checkpoints, bash guards, and live workflow state events.
- **Token-saving conversation management:** collapse older tool results and transcript history into recoverable stubs, then use `recall`/paged history when detail is needed again.
- **Composable interfaces:** use the TUI locally, expose a running agent through HTTP/WebSocket, or register external MCP tools without changing the core harness.
- **Keyboard shortcuts (planned):** profile-driven shortcut customization for default, Vim-like, and browser-like interaction styles.

## Principles

- **Small core, replaceable edges:** the harness owns model turns, tool execution, persistence, workflows, and UDS orchestration; user interfaces and remote integrations stay separate.
- **Local-first and long-running:** Quecto is designed for long-lived sessions on laptops, VPSes, small Linux hosts, and containers, without requiring Node.js, Python, or other application runtimes.
- **Inspectable agent work:** workflows, subagent lifecycle events, paged history, and recoverable context stubs make it possible to supervise and audit long agent runs.
- **Review before trust:** the built-in development workflow expects tests, local review, PR review, and conformance checks rather than treating a single agent pass as sufficient.
- **Protocol over embedding:** Terminal UI, MCP bridge, runtime manager, API gateway, and the agentic harness communicate over UDS/HTTP boundaries instead of sharing UI-specific internals.

Companion crate versions are declared in each package `Cargo.toml`.

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

Some crates also have BDD test targets and package-specific quality scripts. Before requesting review, contributors must run at least two complete built-in Quecto adversarial-review workflow loops and include the evidence in the PR description. See [CONTRIBUTIONS.md](CONTRIBUTIONS.md) and the package READMEs for details.

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
