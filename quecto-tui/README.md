# quecto-tui

A lightweight terminal UI client for `quecto agent --mode uds`.

**Version `0.77.27` (pre-1.0).** The TUI is a UDS bus client of the harness: the
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

Install both binaries from source from the workspace root:

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
| `--model <provider/model>` | Start the spawned agent on this model, in memory only — nothing is written |
| `--effort <level>` | Start the spawned agent on this reasoning-effort level, in memory only |

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
| `Ctrl+D` | Exit: persist, then ask each owned agent to settle its subagents and exit |
| `Ctrl+G` | Jump to the latest conversation output |
| `Ctrl+L` | Open model selector |
| `Ctrl+O` | Toggle tool output expansion |
| `Ctrl+Z` | Suspend the TUI (`fg` to resume) |
| `PageUp` / `PageDown` | Scroll chat |
| `Up` / `Down` | Browse input history |
| `Ctrl+W` | Delete backward through whitespace and one non-whitespace chunk |
| `Alt+D` | Delete forward through non-letters/digits and one Unicode letter/digit word |
| `Ctrl+Y` | Reinsert the latest text removed by `Ctrl+U`, `Ctrl+K`, `Ctrl+W`, or `Alt+D` |
| `Ctrl+Left` / `Ctrl+Right` | Word movement aliases using the existing whitespace boundaries |

Alt/Option and modified-arrow sequences depend on terminal and OS configuration;
Quecto can only handle shortcuts that the terminal sends distinctly.

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
| `/model` | Open the model selector; `Tab` cycles what `Enter` does: use for this session, use and pin as this repo's default (`./.quecto/config.json`), use and pin as the global default (`~/.quecto/config.json`, or the `--config` file the agent was started with) — the harness records it, the toast names the file and reminds you that live tool-policy overlays re-baseline on the next turn |
| `/model <name>` | Switch to a model directly |
| `/clear` | Clear the current conversation |
| `/new` | Start a fresh conversation |
| `/resume` | Open the session picker (`/resume <key>` resumes a key directly). `Tab` moves between **Sessions**, **Scope** (Local Folder / All Folders) and **Search**. The search box asks the harness as you type and matches **the words you type, in any order, literally** — in a session's title, its folder path and its repository name (in Local Folder: the title and the path below the repository root, since every local session shares the rest); a full session ID (key) matches exactly; case is ignored. It is no longer a fuzzy (subsequence) filter: `fxbg` does not find "fix bug" — type `fix bug` or `bug fix` (fuzzy title matching is tracked in #2043). A paste goes to the search box whichever section has the focus: its first line only (the picker says so when more lines were dropped), up to 256 characters — a longer one is refused whole; typing and pasting accept the same characters (anything visible; control, bidi and zero-width characters are dropped; any whitespace — a tab, a no-break space, a Unicode line separator — becomes a space, and a pasted run of it one space, so `fix⇥bug` searches `fix bug`). In a picker too small to have a notice row (e.g. 40×12 beside the agents pane) the `Paste refused…` / `Pasted the first line only` notice is not shown; the box simply keeps its text. `Sessions · Searching…` means the rows still belong to older text: `Enter` then waits for the answer and opens its top match, and the header says so while it waits (`Searching… ⏎ will open the top match`; in a narrower panel `Sessions · ⏎ Searching…`, then `⏎ Sessions…`, down to a bare `⏎` — the `⏎` is on screen at any width the header is). Any other key or click — `Tab`, a cursor move, typing, a paste, a scope change, `Esc` — withdraws that Enter, as do a lost connection and the clock: an Enter is paid only within 5 s of the keypress — a search that is not answered in time is retried once, and text typed ahead of an answer is searched next, but an Enter older than 5 s opens nothing. An Enter is only ever owed to a search of text you typed: while a listing loads (`Sessions · Loading…`, e.g. `/resume` `Enter` `Enter`) it does nothing. `Sessions · No answer` means the search was given up — edit the text or change Scope to retry; `Sessions · Disconnected` means the connection was lost while rows were awaited — the TUI does not reconnect, so restart `quecto-tui` to resume a session (a `/resume <key>` typed while disconnected is refused, not queued). A row's detail line shows its folder, time, what `Enter` does, its `ID`, its repository and which fields matched; "Showing 200 of 5,200 — keep typing to narrow" means the list was cut. Against an older harness (one that rejects the search command itself) the TUI says so once and filters the listed sessions of the scope on screen itself — by title, full session ID and folder path; in Local Folder only the path below the folder all listed sessions share, which stands in for the repository root the listing does not name. It differs from the harness's rule in three cases: a linked worktree outside the repository (their common parent stands in, so the repository's name matches again), every listed session in one single folder (nothing lies below it, so the whole path is matched — more rows than the harness would find, never fewer), and sessions spread only over sub-folders of one sub-folder (that sub-folder's own name matches none of them) |
| `/session` | Show session statistics |
| `/setup` | Ask the agent to walk through quecto setup for this folder/machine (credential, repo overlay, default model, admission broker, container): it submits a walkthrough prompt as your turn; the agent reads the `setup` docs page, reports each area's state, proposes the runbook commands and asks before writing anything. Variants: `/setup model <model-id>`, `/setup admission`, `/setup podman` (alias `container`), `/setup auth`. Master-session only: with a sub-agent focused it refuses with a toast (Esc back to the master, then `/setup` again) and sends nothing to the child |
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
- On exit, `quecto-tui` sends SIGTERM to the spawned agent's leader process
  only and waits for it to settle its own subagents (see "Exit, detach, and
  resume"); it never signals a process group or a descendant.

### Admission waiting indicators

Queued admission waits display as `⏳ Ns` (for example, `⏳ 12s`). The subagent
panel prioritizes this indicator over long child names. The hourglass means
waiting for admission, **not active inference**; brief display during immediate
admission is expected. Selecting or reconnecting to a waiting child restores
its indicator from the child's admission snapshot. Dated cooldown labels count
down from their transition snapshot using local monotonic time. At local expiry
they remain visible as `cooldown elapsed` until an authoritative update arrives;
this presentation does not claim admission availability.

### Coordinator timer

The Coordinator row's timer is the session's cumulative active-processing
time: it runs while the agent is processing (from `agent_start` to
`agent_end`, an abort, an error or a disconnect) and freezes in between, so
idle time between your messages and the agent's wakes never counts and a new
message resumes the frozen value rather than restarting at `0:00` (#1726). It
restarts at `0:00` only at a session boundary: `/new`, `/clear` or a
`/resume` into a different session.

### Subagent transcript freshness

Direct child-socket feeds display live token events. Open subagent feeds also
request committed-ledger catch-up every **2 seconds**, including while idle.
Socketless (root-routed inspection) feeds use this cadence
for automatic transcript refresh; it is **not token streaming**. Only opened
feeds are polled, within the existing warm-feed cap.

Catch-up uses the last applied cursor rather than assuming an earlier request
will be answered. A refused enqueue, lost final hint/response, or refused page
continuation can therefore recover on a subsequent tick once the transport can
accept and answer work—without refocusing or waiting for another hint. This is
not a two-second delivery guarantee under a slow, disconnected or persistently
backpressured transport. It requires an agent that implements ledger `sync`.
A successful sync establishes capability even if a slim state response omits
its optional advertisement. Transcript updates preserve a scrolled-up viewport;
returning to the tail restores following.

### Exit, detach, and resume

Ordinary Ctrl-D, `/exit`, and `/quit` persist the conversation before terminating
TUI-owned agents. Resuming restores transcript history; it does **not** recreate
old children as operational agent-panel rows. Old spawn and tool messages remain
history, not evidence that their processes are alive: a harness-launched
subagent is lifetime-bound to the harness that launched it and ends by itself
when that harness exits or loses its connection to it.

Since epic #1929 the harness owns its own teardown: on SIGTERM it asks every
direct child to shut down over the protocol, each child settles its own
subtree the same way, container environments run their retained kill, and the
harness persists and exits 0 by itself. Its fleet teardown settles direct
children 8 at a time with a 25 s per-child worst case, over at most 3 passes;
a **repeated** SIGTERM inside that work is ignored, and one arriving after
45 s forces the harness out. The TUI's exit path (#1956) therefore signals
**one** process per owned harness and mirrors those numbers:

1. After the snapshots are persisted, SIGTERM to the harness leader pid —
   never `kill(-pgid)`, never a descendant.
2. Wait for that process to exit within the *settle* budget: `ceil(n / 8)`
   batches × 25 s + 5 s to persist and exit, where `n` is the number of
   subagents the roster last showed (30 s for up to 8, 55 s for 9–16,
   80 s — the 3-pass worst case — beyond that or when the roster is unknown).
3. If it is still running, send a **second** SIGTERM — the repeated signal is
   what arms the harness's own 45 s force-exit — and wait those 45 s.
4. Only then SIGKILL that one pid.

If the wait passes ~1 s the TUI shows "waiting for the agent to settle its
subagents… (Ns)" and keeps it updated; the notice is dismissed as soon as the
leaders are gone. A leader that needed the SIGKILL is reported after terminal
cleanup. After each leader exits a read-only canary reads `/proc` once and
reports — never signals — any process still naming the old pid as parent or
process group (skipped if the pid has already been recycled); under the
lifetime binding that list is always empty, and a non-empty one is printed
after terminal cleanup as the evidence. Swarm members and bash tool children
that run in their own process group are, by design, outside the canary's
view. The same leader-only helper serves startup-failure cleanup (which prints "waiting for the agent to exit…"
once on stderr if it takes more than a second).

Use `--detach-on-exit` to leave owned agents running (`--kill-on-exit` is the
default). Externally attached agents are not killed merely because this TUI exits.
Attaching to a running harness (`--socket`) shows that harness's live sub-agent
roster; the TUI keeps no registry of its own. Exit durability and cleanup errors
are reported separately. One `quecto-tui` process holds exactly one harness
connection (#2044): there are no tabs, and a second session means a second
`quecto-tui`.

`$XDG_DATA_HOME/quecto/tui/tab-agent-registry.json` (default
`~/.local/share/quecto/tui/`; and `workspace-manifests.json`
beside it), written by 0.77.25 and earlier, is no longer read or written and can
be deleted (#2044).
