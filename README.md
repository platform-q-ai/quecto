# Quecto

Quecto is a tiny, subagent-first orchestration harness for long-running coding agents. A parent agent stays responsive while delegating focused work to fully operational background subagents, optionally containerised, so research, implementation, review, and verification can happen in parallel without collapsing into one oversized conversation. Those subagents idle at around 12 MB of memory, making delegation cheap enough to be a default operating model rather than a special case.

Quecto’s built-in workflow engine makes that delegation programmable. Instead of relying only on ad-hoc LLM-to-LLM triage, workflows can drive agents through explicit, repeatable processes: spawn reviewers, require checkpoints, run guards, loop fixes through adversarial review, and preserve the evidence needed to resume or audit the work later.

Quecto runs as a small Rust binary with tools, workflows, and the subagent replication system built in. It can run directly as a CLI or REPL, and it can also expose its built-in UDS event bus so external clients, gateways, and tool extensions can integrate without needing to modify the harness.

## Features at a glance

- **Subagent-first orchestration:** keep a parent agent responsive while fully operational child agents handle focused investigation, implementation, review, and verification work.
- **Cheap replication:** run multiple background agents with a small idle memory and CPU footprint, with optional container runtimes when stronger isolation or reproducible environments matter.
- **Built-in workflow engine:** steer agents through explicit feature, refactor, chore, bugfix, and adversarial-review processes with checkpoints, guard commands, and live workflow state.
- **Workspace-aware tools:** give agents shell, file editing, search, docs, recall, workflow, and extension tools rooted in the active workspace and governed by repository hooks.
- **Provider support:** use OpenAI, Anthropic, ChatGPT Codex, or OpenAI-compatible endpoints through the same provider abstraction and credential store.
- **Ultra-long-running sessions:** keep sessions usable over extended work with a configurable, sliding, auto-pruning context window. Older tool results and transcript history collapse into recoverable stubs that the model can retrieve with `recall` when detail is needed again, meaning disruptive manual compaction cycles are no longer required.
- **Composable interfaces:** run `quecto` directly, use the TUI locally, expose a running agent through HTTP/WebSocket, or register external MCP tools over the UDS event bus.

## Principles

- **Subagents by default:** delegation should be cheap enough that independent research, implementation, and review can run outside the parent's main conversation instead of competing for one context window.
- **Predictable process around probabilistic agents:** model judgment remains useful, but workflows, checkpoints, guards, and review loops provide a more inspectable structure for long-running work.
- **Small core, replaceable edges:** the harness owns model turns, tool execution, persistence, workflows, and subagent orchestration; user interfaces and remote integrations stay separate.
- **Local-first and long-running:** Quecto is designed for long-lived sessions on laptops, VPSes, small Linux hosts, and containers, without requiring Node.js, Python, or other application runtimes.
- **Inspectable agent work:** workflows, subagent lifecycle events, paged history, and recoverable context stubs make it possible to supervise and audit long agent runs.
- **Review before trust:** the built-in development workflow expects tests, local review, PR review, and conformance checks rather than treating a single agent pass as sufficient.
- **Protocol over modification:** Terminal UI, MCP bridge, runtime manager, API gateway, and external tools integrate over UDS/HTTP boundaries without needing to share UI internals or modify the harness.

Companion crate versions are declared in each package `Cargo.toml`.

## Workspace projects

| Project | Package / binary | What it is | Status |
|---|---|---|---|
| [Agentic harness](quecto-agentic-harness/) | `quecto-agentic-harness` / `quecto` | The core CLI and UDS agent kernel. | Late Beta |
| [Terminal UI](quecto-tui/) | `quecto-tui` | A lightweight terminal client for the `quecto` UDS agent. | Early Beta |
| [HTTP/WebSocket API](quecto-api/) | `quecto-api` | A REST and WebSocket gateway to a running UDS agent. | Alpha |
| [MCP bridge](quecto-mcp/) | `quecto-mcp` | A standalone UDS-driven extension that exposes remote MCP server tools as Quecto tools. | Alpha |
| [Runtime manager](quecto-runtime-manager/) | `quecto-runtime-manager` | A small HTTP manager for per-session Quecto runtimes. | Alpha |
| [Line/framing I/O](quecto-line-io/) | `quecto-line-io` | Shared bounded JSON/UDS reader and writer library. | Alpha |

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

- Getting started: [quecto-agentic-harness/docs/getting-started.md](quecto-agentic-harness/docs/getting-started.md)
- Harness user guide and CLI/UDS reference: [quecto-agentic-harness/README.md](quecto-agentic-harness/README.md)
- Workflows and templates: [quecto-agentic-harness/docs/workflow.md](quecto-agentic-harness/docs/workflow.md)
- Subagent spawning and control commands: [quecto-agentic-harness/docs/subagents.md](quecto-agentic-harness/docs/subagents.md)
- Sessions, context management, spill, and recall: [quecto-agentic-harness/docs/sessions.md](quecto-agentic-harness/docs/sessions.md)
- Tool policy and command governance: [quecto-agentic-harness/docs/tool-policy.md](quecto-agentic-harness/docs/tool-policy.md)
- Extending Quecto with tools, clients, and model providers: [quecto-agentic-harness/docs/extending-quecto.md](quecto-agentic-harness/docs/extending-quecto.md)
- Extensions and external tools: [quecto-agentic-harness/docs/extensions.md](quecto-agentic-harness/docs/extensions.md)
- Model providers and runtime configuration: [quecto-agentic-harness/docs/runtime-models-providers.md](quecto-agentic-harness/docs/runtime-models-providers.md)
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
