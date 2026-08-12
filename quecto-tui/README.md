# quecto-tui

A lightweight terminal UI client for `quecto agent --mode uds`.

**Version `0.76.7` (pre-1.0).** The TUI is a UDS bus client of the harness: the
wire protocol and session ownership live in `quecto`, so most breaking risk is
upstream. This crate stays on `0.y` until feature-oriented presentation boundaries and
public surface (flags, slash commands, attach/spawn) meet the bar for a deliberate
`1.0` freeze — not because the client is unused or unready for daily work.

The TUI is a client. The Quecto kernel is the root `quecto` binary running
`quecto agent --mode uds`; it owns the model session, tools, credentials,
workflow state, and Unix socket. When no `--socket` is passed, `quecto-tui`
starts that kernel by executing `quecto`, so `quecto` must be on `PATH`.

`quecto-tui` is a workspace member in this repository. It either:

- spawns `quecto agent --mode uds` for you and connects automatically, or
- connects to an already-running agent via `--socket <path>`.

## Build and install

Install both binaries from the workspace root:

```bash
# From the workspace root:
cargo install --path quecto-agentic-harness
cargo install --path quecto-tui
```

Then start the TUI and let it spawn the kernel automatically:

```bash
quecto-tui
```

Workflow-driven launch:

```bash
quecto-tui --workflow --workflow-guards
```

For the repository lead-developer prompt, use `scripts/run-tui.sh`. During TUI
development, `scripts/dev-tui.sh` incrementally rebuilds and restarts it when
workspace files change; this requires `cargo-watch`.

## Architecture direction

`quecto-tui` is being refactored as a feature-oriented presentation adapter for
harness-facing capabilities: conversation, sessions, agents, workflow,
inference, workspace, protocol, shell, and reusable components. The current
architecture direction is documented in
[`docs/feature-oriented-presentation-architecture.md`](docs/feature-oriented-presentation-architecture.md);
the older Clean Architecture target-model note is superseded historical context.

## Run from the workspace

If you do not want to install, either make Cargo's build output visible on
`PATH` so `quecto-tui` can spawn `target/debug/quecto`:

```bash
cargo build -p quecto-agentic-harness -p quecto-tui
PATH="$PWD/target/debug:$PATH" cargo run -p quecto-tui --
```

Or run the kernel explicitly and connect the TUI to its socket from another
terminal:

```bash
cargo run -p quecto-agentic-harness -- agent --mode uds --socket /tmp/quecto.sock --persist
cargo run -p quecto-tui -- --socket /tmp/quecto.sock
```

`--persist` keeps the kernel alive when clients disconnect. Omit it if you want
the kernel to exit automatically after the last client disconnects.

## Spawned-agent flags

When `quecto-tui` spawns the agent for you, it can forward these flags:

| Flag | Description |
|---|---|
| `--socket <path>` | Connect to an existing UDS agent instead of spawning one |
| `--workflow` | Start the spawned agent in workflow-driven mode immediately |
| `--workflow-guards` | Enable workflow bash guards for the spawned agent; does not by itself force workflow prompt injection |
| `--no-workflow` | Disable workflow tool/state/prompt for the spawned agent |
| `--system <prompt>` | Pass a custom system prompt to the spawned agent |
| `--config <path>` | Use an alternate quecto config file when spawning the agent |
| `--no-sandbox` | Spawn the agent with filesystem sandboxing disabled |

By default, the spawned UDS agent has the workflow tool available but dormant:
you can talk normally, then ask the model to select a workflow template when you
want one. If you pass `--workflow` without `--system`, `quecto-tui` injects a
default coding-assistant system prompt that tells the agent to use the workflow
tool immediately. An explicit `--system` value overrides that default.

## Startup errors

If the spawned agent exits before announcing its UDS socket, `quecto-tui`
prints a redacted snippet of the agent's stderr context (for example, missing
provider credentials) instead of only reporting that startup failed.

## Cold-binary first launch

The **first launch** right after `cargo install` can be slower than usual: the
freshly written `quecto` binary is cold in the OS page cache, so the kernel can
take longer to start. To absorb this, `quecto-tui` waits up to **30s** (a 30s
spawn-readiness deadline) for the agent to announce its socket before failing,
and on timeout it suggests warming the binary with `quecto --version` and
retrying. `scripts/run-tui.sh` pre-warms `quecto --version` before launching the
TUI so the cold cost is paid up front; direct `quecto-tui` invocations rely on
the 30s deadline instead.

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

## Mouse and markdown links

Safe `http(s)` markdown links in chat are emitted as real **OSC 8** hyperlinks
(blue + underline). The TUI enables DEC/SGR mouse reporting so the wheel scrolls
chat and drag selects text; while that capture is on, a plain click starts
selection instead of opening the link.

To open a link in the browser, use your terminal’s mouse-capture bypass:

| Gesture | Typical terminals |
|---|---|
| **`Shift+click`** | Alacritty and many other DEC-mouse terminals |
| `Ctrl+click` / `Cmd+click` | Some terminals use a different modifier |

The same note appears in the in-app `/help` (also `/hotkeys`) listing. The
modifier is a terminal feature, not a Quecto keybinding — if plain click ever
opens links natively, that help line is the single place to update first.

## Slash commands

| Command | Action |
|---|---|
| `/model` | Open the model selector |
| `/model <name>` | Switch to a model directly |
| `/clear` | Clear the current conversation |
| `/new` | Start a fresh conversation |
| `/session` | Show session statistics |
| `/workflow-auto` | Toggle core workflow auto-continue |
| `/workflow-nudge` | Toggle core workflow completion nudge |
| `/help` or `/hotkeys` | Show built-in help |
| `/quit` or `/exit` | Exit the TUI |

Autocomplete includes the built-in slash commands, including `/hotkeys` as an
alias for `/help`.

## Notes

- The library crate exposes the feature-oriented modules `agents`, `components`,
  `conversation`, `inference`, `protocol`, `sessions`, `shell`, `workflow`, and
  `workspace` (#1257 Phase 6). Internal TUI modules are reached through these
  owners, e.g. `quecto_tui::shell::cli` or `quecto_tui::shell::app`; root-level
  `quecto_tui::client`-style shims are intentionally not part of the public API.
- Auto-discovered socket paths are validated and must be real Unix sockets under
  canonical `/tmp`, `$TMPDIR`, `$XDG_RUNTIME_DIR`, or `$HOME` roots.
- On exit, `quecto-tui` terminates the spawned agent process group so child
  agents are cleaned up too.
