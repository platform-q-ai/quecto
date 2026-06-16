# quecto-tui

A lightweight terminal UI client for `quecto agent --mode uds`.

`quecto-tui` is a workspace member in this repository. It either:

- spawns `quecto agent --mode uds` for you and connects automatically, or
- connects to an already-running agent via `--socket <path>`.

## Run it

From the workspace root:

```bash
cargo run -p quecto-tui -- --workflow --workflow-guards
```

Connect to an existing agent instead of spawning one:

```bash
cargo run -p quecto-tui -- --socket /tmp/agent.sock
```

## Spawned-agent flags

When `quecto-tui` spawns the agent for you, it can forward these flags:

| Flag | Description |
|---|---|
| `--socket <path>` | Connect to an existing UDS agent instead of spawning one |
| `--workflow` | Start the spawned agent with workflow support enabled |
| `--workflow-guards` | Start the spawned agent with workflow bash guards enabled |
| `--no-workflow` | Disable workflow flags for the spawned agent |
| `--system <prompt>` | Pass a custom system prompt to the spawned agent |
| `--config <path>` | Use an alternate quecto config file when spawning the agent |
| `--no-sandbox` | Spawn the agent with filesystem sandboxing disabled |
| `--network` | Allow outbound network access for bash in the spawned agent |

If you pass `--workflow` without `--system`, `quecto-tui` injects a default
coding-assistant system prompt that tells the agent to use the workflow tool.
An explicit `--system` value overrides that default.

## Startup errors

If the spawned agent exits before announcing its UDS socket, `quecto-tui`
prints a redacted snippet of the agent's stderr context (for example, missing
provider credentials) instead of only reporting that startup failed.

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| `Enter` | Send message |
| `Shift+Enter` or `Alt+Enter` | Insert newline |
| `Escape` | Abort the active agent run, or clear the editor if idle |
| `Ctrl+C` | Clear the editor first; if the editor is empty, abort the active run |
| `Ctrl+D` | Exit immediately |
| `Ctrl+L` | Open model selector |
| `Ctrl+O` | Toggle tool output expansion |
| `Ctrl+Z` | Suspend the TUI (`fg` to resume) |
| `PageUp` / `PageDown` | Scroll chat |
| `Up` / `Down` | Browse input history |

## Slash commands

| Command | Action |
|---|---|
| `/model` | Open the model selector |
| `/model <name>` | Switch to a model directly |
| `/clear` | Clear the current conversation |
| `/new` | Start a fresh conversation |
| `/session` | Show session statistics |
| `/workflow-auto` | Toggle workflow auto-continue |
| `/workflow-nudge` | Toggle workflow completion nudge |
| `/help` or `/hotkeys` | Show built-in help |
| `/quit` or `/exit` | Exit the TUI |

Autocomplete includes the built-in slash commands, including `/hotkeys` as an
alias for `/help`.

## Notes

- The library crate exposes only Clean Architecture layer modules
  (`application`, `domain`, `infrastructure`, `interface`). Internal TUI modules
  are reached through those layers, e.g. `quecto_tui::infrastructure::client` or
  `quecto_tui::interface::app`; root-level `quecto_tui::client`-style shims are
  intentionally not part of the public API.
- Auto-discovered socket paths are validated and must be real Unix sockets under
  canonical `/tmp`, `$TMPDIR`, `$XDG_RUNTIME_DIR`, or `$HOME` roots.
- On exit, `quecto-tui` terminates the spawned agent process group so child
  agents are cleaned up too.
