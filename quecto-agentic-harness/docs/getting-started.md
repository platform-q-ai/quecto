# Getting Started

Minimal path from zero to a working Quecto agent. Config is optional — defaults
apply with no `~/.quecto/config.json`.

## 1. Install

From the workspace root:

```bash
cargo install --path quecto-agentic-harness
cargo install --path quecto-tui   # optional terminal UI
```

Ensure `quecto` is on your `PATH` (`~/.cargo/bin` after install).

## 2. Authenticate

Pick a provider and store a credential:

```bash
# API key
quecto auth login --provider openai --token sk-proj-your-key
# or
quecto auth login --provider anthropic --token sk-ant-your-key

# Or OAuth (browser)
quecto auth login --provider openai --oauth
quecto auth login --provider anthropic --oauth
```

Check status anytime:

```bash
quecto auth status
```

Env vars also work: `OPENAI_API_KEY` / `ANTHROPIC_API_KEY`.

## 3. Run

```bash
# One-shot
quecto agent -m "Hello, what can you do?"

# Interactive REPL
quecto

# Terminal UI (spawns a UDS agent for you)
quecto-tui
```

## Next steps

| Need | Where |
|---|---|
| Flags, config, tools, security | [README](../README.md) |
| Models / `models.json` | [Runtime models & providers](runtime-models-providers.md) |
| UDS protocol for custom clients | [UDS protocol](uds-protocol.md) |
| Subagents | [Subagents](subagents.md) |
| Workflows | [Workflow](workflow.md) |
