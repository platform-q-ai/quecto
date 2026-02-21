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
quecto auth login --provider openai --token sk-proj-your-key
quecto auth login --provider anthropic --token sk-ant-your-key
quecto auth status
quecto auth logout --provider openai
```

Credentials are stored in `~/.quecto/credentials.json`. The credential store takes priority over keys in `config.json`.

### `quecto skills` — Manage skills

Skills are markdown files that extend the agent's system prompt with domain knowledge or instructions.

```bash
quecto skills list
quecto skills remove my-skill
```

To add a skill, create a directory under your workspace:

```
~/.quecto/workspace/skills/my-skill/SKILL.md
```

The content of `SKILL.md` is prepended to the system prompt on every agent run. Multiple skills are concatenated.

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

Quecto supports OpenAI and Anthropic as LLM providers. If both are configured, it uses automatic fallback:

- Tries the primary provider first
- On rate-limit or server errors, falls back to the secondary provider
- Authentication errors (wrong API key) do not trigger fallback
- Providers enter a cooldown period after failures

API key resolution order: credential store (`quecto auth login`) > config file > environment variable.

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
         "allow_from": ["your-telegram-user-id"]
       }
     }
   }
   ```
4. Run `quecto gateway`

Set `allow_from` to an empty array `[]` to allow all users (not recommended for public bots).

The gateway shuts down cleanly on Ctrl+C (SIGINT). The Telegram polling loop exits and all in-flight tasks are dropped without error.

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
    skills/                # Skill definitions
      my-skill/
        SKILL.md
    HEARTBEAT.md           # Periodic task list (for gateway heartbeat)
    ...                    # Agent working directory (files created by the agent)
```

## License

MIT
