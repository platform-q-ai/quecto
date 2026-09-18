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

`quecto config set --global …` edits the global file the same way: one key's
value changes, every other key stays (the file is re-laid-out as pretty JSON).
`quecto config get` hides API keys and tokens unless you pass
`--show-secrets`. A pre-#2024
`./config.json` in the working directory is no longer loaded — `quecto status`
warns while one exists; move its settings to `./.quecto/config.json`. See the
README's
[Configuration discovery and precedence](../README.md#configuration-discovery-and-precedence).

## Set up by an agent (or by hand, in the same order)

Every setup step is a `quecto` command that prints the file it wrote and
exits 0, or prints why it refused and exits 1 with the file unchanged. An
agent with the `docs` tool starts at `docs {"name": "setup"}`, which routes
each situation to one runbook page; these are the same commands:

| Situation | Do | Verify (expected) | Rollback |
|---|---|---|---|
| Credential | `quecto auth login --provider openai --token <key>` (from an agent always `--token`: without it the browser flow blocks) | `quecto auth status` → `openai (token) — active` | `quecto auth logout --provider openai` |
| Default model for this repo | `quecto config set agents.defaults.model '"openai-api/gpt-5.6-luna"'` | `quecto config get --effective agents.defaults.model` → the id; `quecto agent --no-session -m "Reply with exactly OK"` → `OK` | `quecto config unset agents.defaults.model` |
| Default model everywhere | `quecto config set --global agents.defaults.model '"…"'` | `quecto status` → `Model: …` | `quecto config unset --global agents.defaults.model` |
| Admission broker (one per host) | `quecto config set --global admission '{…}'`, `quecto admission-broker install-service` | `quecto admission-broker status` → `{"directory":…,"epoch":1,"journal_healthy":true,…}` | `quecto admission-broker uninstall-service`, `quecto config unset --global admission` |
| Container for this repo | `quecto container init`, then the `podman build …` line it prints | `quecto container status` → `ready: …`; `quecto container doctor` → no `✗` line, exit 0 | `quecto config unset --local container_configs.standard`, `rm -r .quecto/containers/standard` |

Order on a fresh machine: credential → (global model) → repo overlay →
admission → container, then `quecto status` once more. Details, expected
outputs and failure tables: the [README](../README.md#commands),
[inference-admission.md](inference-admission.md#activation-rollback-and-quarantine-runbook)
and [container-runtimes.md](../../docs/container-runtimes.md#the-standard-container-quecto-container-init-2024-s4e).

## Next steps

| Need | Where |
|---|---|
| Flags, config, tools, security | [README](../README.md) |
| Models / `models.json` | [Runtime models & providers](runtime-models-providers.md) |
| UDS protocol for custom clients | [UDS protocol](uds-protocol.md) |
| Subagents | [Subagents](subagents.md) |
| Workflows | [Workflow](workflow.md) |
