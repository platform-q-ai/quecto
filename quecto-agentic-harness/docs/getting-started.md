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

# Login, setup, and configuration shell
quecto

# Terminal UI (spawns a UDS agent for you)
quecto-tui
```

## Project-local configuration

Your global settings live in `~/.quecto/config.json`. A project can carry a
small overlay at `./.quecto/config.json` in the directory you launch quecto
from; it is merged over the global file section by section (`agents.defaults`,
`tools`, `workflow`, `container_configs`), so a repository never has to copy
your providers, tool policy or admission settings — those sections are
global-only and refused in an overlay. Parent directories are not searched.

An overlay is applied only once you have approved its exact content, so a
checked-out repository cannot change your defaults behind your back:

```bash
cd ~/src/app
quecto config set agents.defaults.model '"openai-api/gpt-5.5"'   # writes and trusts ./.quecto/config.json
quecto config trust           # approve an overlay someone else wrote, after reviewing it
quecto config get --effective # what a run here will actually use
quecto status                 # both files, and whether the overlay is trusted
```

`quecto config set --global …` edits the global file the same way: one key
changes, everything else in the file stays as you wrote it. A pre-#2024
`./config.json` in the working directory is no longer loaded — `quecto status`
warns while one exists; move its settings to `./.quecto/config.json`. See the
README's
[Configuration discovery and precedence](../README.md#configuration-discovery-and-precedence).

## Next steps

| Need | Where |
|---|---|
| Flags, config, tools, security | [README](../README.md) |
| Models / `models.json` | [Runtime models & providers](runtime-models-providers.md) |
| UDS protocol for custom clients | [UDS protocol](uds-protocol.md) |
| Subagents | [Subagents](subagents.md) |
| Workflows | [Workflow](workflow.md) |
