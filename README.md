# Quecto

A single-binary personal AI assistant that runs on minimal Linux systems. Quecto receives messages via Telegram or the command line, routes them through an LLM (OpenAI or Anthropic), executes tools (shell commands, file operations, web search, scheduled tasks), and persists conversations to disk.

Built in Rust. No runtime dependencies. Runs on a VPS, Raspberry Pi, or container.

## Quick Start

```bash
# Install
cargo install --path .

# Set up config and workspace
quecto onboard

# Store your API key
quecto auth login --provider openai --token sk-proj-your-key

# Talk to the agent (one-shot)
quecto agent -m "Hello, what can you do?"

# Or start an interactive session
quecto
```

## Commands

### `quecto` — Interactive REPL

When run with no arguments, quecto enters an interactive read-eval-print loop:

```bash
quecto
```

The REPL reads input line by line, sends each to the LLM agent, prints the response, and repeats.

| Flag | Description |
|---|---|
| `-s` / `--session` | Session name for persistence. Default: `repl:repl_default`. Use `-` for ephemeral |
| `--system` | System prompt prepended to each turn (not persisted) |
| `--model` | Override the default model from config |

REPL commands: `/help` (show commands), `/clear` (reset history), `/exit` or `/quit` (exit). Ctrl+D (EOF) also exits cleanly.

Piped input is supported for scripting: `echo "hello" | quecto`.

### `quecto agent` — Talk to the agent

```bash
quecto agent -m "Write a Python script that generates primes"
```

| Flag | Required | Description |
|---|---|---|
| `-m` / `--message` | Yes | The message to send |
| `-s` / `--session` | No | Session name for persistence. Omit for `cli:default`. Use `-` for ephemeral |
| `--system` | No | System prompt prepended to conversation |
| `--model` | No | Override model (default: `gpt-5.2`) |
| `--max-iterations` | No | Max tool call rounds before stopping |
| `--max-time` | No | Wall-clock timeout in seconds (exit code 2 on timeout) |

**Sessions** persist conversation history so the agent remembers context across runs:

```bash
quecto agent -s myproject -m "I'm working on a web scraper in Python"
quecto agent -s myproject -m "Add error handling to what we discussed"
```

Use `-s -` for one-off questions that don't need history.

### `quecto gateway` — Run as a Telegram bot

Starts a long-running service that polls Telegram for messages and responds through the agent. The gateway has access to additional tools not available in CLI mode (web search, cron scheduling, subagent spawning, message sending).

```bash
quecto gateway
```

Requires Telegram configuration in `config.json` (see [Configuration](#configuration)).

#### Telegram bot commands

The gateway intercepts the following bot commands directly, without routing them through the agent:

| Command | Response |
|---|---|
| `/start` | Welcome message |
| `/help` | Lists available bot commands |
| `/status` | Shows current model and Telegram status |

Any other message (including unknown `/commands`) is forwarded to the agent for processing.

### `quecto auth` — Manage API keys

```bash
# Pass token directly
quecto auth login --provider openai --token sk-proj-your-key

# Interactive: prompts you to paste the token
quecto auth login --provider anthropic

# OAuth browser flow
quecto auth login --provider openai --oauth

# Device code flow (for headless environments)
quecto auth login --provider openai --device-code

quecto auth status
quecto auth logout --provider openai
```

| Subcommand | Flags | Description |
|---|---|---|
| `auth login` | `--provider <name>` (required) | Authenticate with a provider |
| | `--token <key>` | Pass token directly (skips interactive prompt) |
| | `--oauth` | Initiate OAuth browser-based login flow |
| | `--device-code` | Initiate device code flow for headless environments |
| `auth logout` | `--provider <name>` | Remove a stored credential |
| `auth status` | | List all stored credentials with status |

Credentials are stored in `~/.quecto/credentials.json`. The credential store takes priority over keys in `config.json`.

### `quecto skills` — Manage skills

Skills are SKILL.md files with YAML frontmatter that extend the agent's system prompt with domain knowledge or instructions.

```bash
quecto skills list       # Shows name and description for each skill
quecto skills remove my-skill
```

To add a skill, create a directory under your workspace with a `SKILL.md` file:

```
~/.quecto/workspace/skills/my-skill/SKILL.md
```

The `SKILL.md` file must contain YAML frontmatter with `name` and `description` fields. The body content (everything after the closing `---`) is prepended to the system prompt on every agent run. Multiple skills are concatenated.

```markdown
---
name: my-skill
description: Short description of what this skill does
license: MIT                    # optional
compatibility: opencode         # optional
metadata:                       # optional
  audience: developers
---
You are an expert at ...

## Instructions
- Do this
- Do that
```

**Frontmatter rules:**
- `name` and `description` are required (description max 1024 chars)
- `name` must match the directory name
- Names must be lowercase alphanumeric with hyphens only, 1–64 chars (e.g. `code-review`, `git-release`)
- Skills with missing or invalid frontmatter are silently skipped

### `quecto status` — Check configuration

Shows the current config, workspace path, model, API key status, and Telegram/heartbeat settings.

```bash
quecto status
```

### `quecto onboard` — First-time setup

Creates the default config file and workspace directory structure:

```
~/.quecto/
  config.json
  workspace/
    AGENTS.md
    IDENTITY.md
    SOUL.md
    TOOLS.md
    USER.md
```

### `quecto help` — Show usage

Prints a summary of all available commands.

```bash
quecto help
```

Also available as `quecto --help` or `quecto -h`.

### `quecto version` — Show version

Prints the version number.

```bash
quecto version
```

Also available as `quecto --version` or `quecto -v`.

## Configuration

Config file: `~/.quecto/config.json`

```json
{
  "agents": {
    "defaults": {
      "model": "gpt-5.2",
      "workspace": "~/Documents/quecto-workspace",
      "max_tokens": 8192,
      "max_tool_iterations": 20,
      "restrict_to_workspace": true
    }
  },
  "providers": {
    "openai": {
      "api_key": "sk-proj-..."
    },
    "anthropic": {
      "api_key": "sk-ant-..."
    }
  },
  "channels": {
    "telegram": {
      "enabled": true,
      "token": "your-bot-token-from-botfather",
      "api_base": "https://api.telegram.org",
      "allow_from": ["123456789"]
    }
  },
  "tools": {
    "web": {
      "brave": {
        "enabled": true,
        "api_key": "your-brave-key",
        "max_results": 5
      },
      "duckduckgo": {
        "enabled": true,
        "max_results": 5
      }
    }
  },
  "heartbeat": {
    "enabled": true,
    "interval": 30
  }
}
```

All fields are optional. An empty `{}` is valid — everything uses sensible defaults.

### Environment variable overrides

| Variable | Overrides |
|---|---|
| `QUECTO_BASE_DIR` | Base directory (default `~/.quecto`) |
| `QUECTO_AGENTS_DEFAULTS_MODEL` | `agents.defaults.model` |
| `QUECTO_AGENTS_DEFAULTS_MAX_TOKENS` | `agents.defaults.max_tokens` |
| `QUECTO_AGENTS_DEFAULTS_TEMPERATURE` | `agents.defaults.temperature` |
| `QUECTO_AGENTS_DEFAULTS_WORKSPACE` | `agents.defaults.workspace` |
| `QUECTO_PROVIDERS_OPENAI_API_KEY` | `providers.openai.api_key` |
| `QUECTO_PROVIDERS_ANTHROPIC_API_KEY` | `providers.anthropic.api_key` |

## Tools

The agent has access to tools it can call autonomously to accomplish tasks.

### CLI mode tools

| Tool | Description |
|---|---|
| `exec` | Execute a shell command (30s timeout, dangerous commands blocked) |
| `read_file` | Read file contents |
| `write_file` | Create or overwrite a file (auto-creates parent directories) |
| `edit_file` | Replace a substring in a file |
| `append_file` | Append content to a file |
| `list_dir` | List directory contents |

### Gateway-only tools

These are available when running `quecto gateway` but not in CLI or REPL mode:

| Tool | Description |
|---|---|
| `web_search` | Search the web via Brave Search or DuckDuckGo |
| `cron` | Manage scheduled tasks (add, remove, list, enable, disable) |
| `spawn` | Spawn a background subagent for long-running tasks |
| `message` | Send a message to the user's channel |

## Security

The agent operates inside a sandbox:

- **Workspace restriction**: When `restrict_to_workspace` is `true` (default), all file operations are confined to the workspace directory. Symlinks pointing outside are blocked. Path traversal (`../`) is caught.
- **Dangerous commands blocked**: `rm -rf /`, `mkfs`, `dd`, `shutdown`, `reboot`, `chmod -R 777 /`, fork bombs, and pipe-to-shell patterns (`curl|sh`) are always blocked regardless of other settings.
- **Environment isolation**: `QUECTO_*` environment variables (including API keys) are stripped from child processes spawned by the `exec` tool.

## Provider Fallback

Quecto supports OpenAI and Anthropic as LLM providers. Both providers support SSE streaming (`chat_stream()`) for incremental response assembly, with automatic fallback to non-streaming mode. If both are configured, it uses automatic fallback:

- Tries the primary provider first
- On rate-limit or server errors, falls back to the secondary provider
- Authentication errors (wrong API key) do not trigger fallback
- Providers enter a cooldown period after failures

API key resolution order: credential store (`quecto auth login`) > config file > environment variable.

## Health Endpoints

When running as a gateway, Quecto exposes HTTP health endpoints for monitoring:

| Endpoint | Description | Response |
|---|---|---|
| `GET /health` | Liveness check | Always `200 OK` with `{"status":"ok"}` |
| `GET /ready` | Readiness check | `200 OK` with `{"ready":true}` if providers available, `503` with `{"ready":false}` otherwise |

The health server uses raw tokio TCP (no hyper/axum) for minimal binary footprint.

## Telegram Setup

1. Create a bot with [@BotFather](https://t.me/BotFather) on Telegram
2. Copy the bot token
3. Add to your config:
   ```json
    {
      "channels": {
        "telegram": {
          "enabled": true,
          "token": "your-bot-token",
          "api_base": "https://api.telegram.org",
          "allow_from": ["your-telegram-user-id"]
        }
      }
    }
   ```
4. Run `quecto gateway`

Set `allow_from` to an empty array `[]` to allow all users (not recommended for public bots).

Set `api_base` only when you need to override the Telegram API endpoint (for example, local integration tests with a mock server). Leave it as the default `https://api.telegram.org` for normal use.

The gateway shuts down cleanly on Ctrl+C (SIGINT). The Telegram polling loop exits and all in-flight tasks are dropped without error.

## Testing

```bash
# Core suite (no real provider calls)
cargo test --test bdd

# Real-LLM smoke subset (CI-sized)
timeout 5m env QUECTO_REAL_LLM=1 QUECTO_TAG=real-llm-smoke cargo test --test bdd

# Real-LLM full suite
timeout 5m env QUECTO_REAL_LLM=1 QUECTO_TAG=real-llm cargo test --test bdd
```

`scripts/pre-push.sh` now runs both default BDD coverage and the full real-LLM suite (tagged `@real-llm`) with timeouts, caches successful runs per `HEAD` commit + script hash, and writes a full log to `.git/pre-push.last.log`.

Pre-push controls:
- `QUECTO_E2E_TIMEOUT` for the default BDD timeout (default `5m`)
- `QUECTO_REAL_LLM_TIMEOUT` for the real-LLM timeout (default `5m`)
- `QUECTO_PREPUSH_FORCE=1` to bypass cache and rerun all checks

## Directory Structure

```
~/.quecto/
  config.json              # Main configuration
  credentials.json         # Stored API tokens (from quecto auth)
  sessions/                # Persisted conversation history
    cli_default.json
    repl_repl_default.json
    telegram_123456.json
  cron/
    jobs.json              # Scheduled task definitions
  workspace/
    skills/                # Skill definitions (YAML frontmatter required)
      my-skill/
        SKILL.md
    HEARTBEAT.md           # Periodic task list (for gateway heartbeat)
    ...                    # Agent working directory (files created by the agent)
```

## License

MIT
