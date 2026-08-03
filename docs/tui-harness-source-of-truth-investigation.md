# Investigation: TUI values that should come from the agentic harness

Date: 2026-07-29
Branch: `chore/docker-harness-local-tui`

## Prompt / goal

The TUI is a view into a running `quecto-agentic-harness` instance. Values that describe the running agent/session should not be derived from the TUI process environment. In particular, the footer's current working directory and git branch should reflect the harness session workspace, not the TUI process working folder.

This investigation records what currently happens, a plan for moving cwd/branch authority into the harness protocol, and other TUI state that should be treated the same way.

## Current findings

### TUI currently owns footer cwd and git branch

Relevant files:

- `quecto-tui/src/components/footer.rs`
  - `Footer::new()` reads `std::env::current_dir()` once and caches it as `pwd`.
  - It also does local `HOME` abbreviation.
  - `Footer::render()` appends the locally tracked branch to that cached pwd.
- `quecto-tui/src/shell/app.rs`
  - `App::new()` reads `std::env::current_dir()` into `git_repo` and reads a git branch using TUI-local helpers.
  - `start_git_branch_refresh()` periodically polls `.git/HEAD` from the TUI process-side repository path.
  - `start_files_autocomplete_load()` also scans `std::env::current_dir()` for file completions.
- `quecto-tui/src/workspace/controller_git.rs`
  - TUI-local `.git/HEAD` discovery and sanitisation.
  - Includes polling interval and gitdir resolution logic.
- `quecto-tui/src/workspace/controller_workspace.rs`
  - Stores `git_branch` and `git_repo` as TUI-owned workspace flow state.

Implication: when TUI and harness run in different process cwd contexts (for example a Docker/local TUI split, attaching to an existing UDS socket, or a future remote harness), the footer can show the wrong folder/branch because it is reading the viewer's environment rather than the agent session's workspace.

### Harness already has the right conceptual source of truth, but does not expose it over UDS state

Relevant files:

- `quecto-agentic-harness/src/interface/shared.rs`
  - Agent workspace resolution is centralised here; tests assert sandbox/no-sandbox workspace behaviour.
- `quecto-agentic-harness/src/interface/cli/agent.rs`
  - Captures the process cwd while building/running the agent.
- `quecto-agentic-harness/src/infrastructure/tools/*`
  - Tools resolve paths relative to the harness workspace/cwd, not the TUI.
- `quecto-agentic-harness/src/interface/cli/protocol.rs`
  - `SessionState` is the current `get_state` response payload.
  - It includes model, streaming status, session key, message counts, context window, effort, workflow, execution, sync.
  - It does **not** include workspace/cwd/repo/branch fields today.
- `quecto-agentic-harness/src/interface/cli/uds_query.rs`
  - `get_state` builds the `SessionState` returned to clients.
- `quecto-agentic-harness/src/interface/cli/uds_snapshots.rs`
  - Busy-connect snapshot path also serves a state snapshot; this needs parity with live `get_state`.
- `quecto-agentic-harness/src/interface/repl/progress.rs`
  - The REPL path has independent cwd + git branch header logic, showing there is prior art, but it is not the UDS/TUI contract.

Implication: the harness can determine the authoritative workspace, but the UDS protocol needs an explicit field before the TUI can consume it.

## Proposed implementation plan for cwd + branch

### 1. Extend the harness protocol with typed workspace metadata

Add a serializable workspace object to `SessionState`, for example:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceState {
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
}
```

Then add to `SessionState`:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub workspace: Option<WorkspaceState>,
```

Notes:

- Keep the field optional/defaulted for wire compatibility with older harnesses.
- Prefer a nested object over top-level `cwd`/`gitBranch` so future workspace fields do not clutter `SessionState`.
- `cwd` should be the harness workspace path tools resolve relative to.
- `display_cwd` may be pre-abbreviated by the harness, or the TUI can continue presentation-only home abbreviation after receiving `cwd`.
- `repo_root` is useful separately from cwd for file discovery and branch polling if we keep any temporary client-side fallback.
- `git_branch` should be the current branch as seen from the harness workspace/repo, sanitized by harness-side display rules.

### 2. Populate workspace metadata in all harness state paths

Update:

- `AgentSession::state_snapshot(...)` in `quecto-agentic-harness/src/interface/cli/uds_session.rs` to accept workspace metadata or a `WorkspaceState` argument.
- Live `get_state` in `quecto-agentic-harness/src/interface/cli/uds_query.rs`.
- Busy-connect snapshot creation/refresh in `quecto-agentic-harness/src/interface/cli/uds_multi.rs`, `uds_lifecycle.rs`, and `uds_snapshots.rs` call sites.

Important invariant: a client attaching while the harness is busy must see the same workspace shape as a normal `get_state` response.

Branch refresh strategy:

- Initial implementation can read git state when producing/refeshing state snapshots.
- If branch changes while idle must remain visible promptly, put the polling or event generation on the harness side, not the TUI side.
- A small harness-side poll cadence equivalent to today's TUI `GIT_BRANCH_POLL_INTERVAL` is acceptable, but should update the published state snapshot and/or emit a state/workspace event.

### 3. Add TUI protocol parsing

Update `quecto-tui/src/protocol/state_payloads.rs`:

- Add `WorkspaceSnapshot` / `GetStateWorkspaceFields` with optional `cwd`, `display_cwd`, `repo_root`, `git_branch`.
- Parse from `data["workspace"]`, sanitizing strings with the existing sanitizer.
- Preserve compatibility: missing workspace should parse to `None` and leave existing fallback behaviour temporarily intact.

### 4. Make the footer display harness workspace fields

Update `quecto-tui/src/components/footer.rs`:

- Stop reading `std::env::current_dir()` in `Footer::new()` for the primary display value.
- Add setters such as `set_workspace_display(path: Option<String>)` and keep `set_git_branch` or replace with a single `set_workspace(...)` method.
- Default display can be `?` or a deliberately labelled fallback until a first `get_state` arrives.

Update `App::handle_get_state(...)`:

- Apply parsed workspace fields to the master session footer.
- Update any retained master session workspace state from the harness, not from TUI cwd.

Subagent note:

- Current subagent `SessionView::new(git_branch)` receives the master `workspace.git_branch`. If child agents can run in different workspaces, their own `get_state` / subagent state should eventually carry workspace too. If they cannot, cloning the master harness workspace is acceptable but should be explicit.

### 5. Remove or downgrade TUI git polling

Once the harness workspace field is available:

- Remove `App::start_git_branch_refresh()` and the event-loop git polling path as the primary mechanism.
- Keep `workspace/controller_git.rs` only if needed for tests or as a short-lived compatibility fallback for older harnesses.
- If a fallback remains, label it as compatibility only and prefer harness fields whenever present.

### 6. Address file autocomplete as follow-up

`start_files_autocomplete_load()` currently scans the TUI cwd. That is the same class of bug as the footer cwd/branch.

Options:

1. Short-term: use harness-provided `workspace.cwd` / `repo_root` as the path passed to `list_workspace_files()`.
2. Preferred: add a harness command such as `list_workspace_files` or `workspace_search` so file discovery, git safety configuration, sandboxing, remote execution, and path visibility all live on the harness side.

The preferred command avoids the TUI running `git` locally for a workspace that may not exist locally.

## Tests to add/change

### Harness

- `SessionState` serialization includes `workspace` with `cwd` and optional `gitBranch`.
- `get_state` live query returns workspace metadata.
- Busy-connect snapshot returns the same workspace metadata.
- Workspace strings are sanitized / bounded for display safety.
- Branch changes are reflected by harness-side refresh if prompt requirements need idle updates.

### TUI

- `parse_get_state` extracts workspace fields.
- `Footer::new()` no longer depends on process cwd for the authoritative value.
- Applying `get_state` updates footer path and branch.
- In a test where TUI cwd differs from harness cwd, the footer shows the harness cwd/branch.
- Older harness compatibility: missing `workspace` does not panic and falls back gracefully.
- File autocomplete uses harness workspace path or harness file-list command, not TUI cwd.

## Other TUI values that should come from the harness

The rule of thumb: if the value describes agent/session/runtime state, harness is authoritative. If it describes presentation/input state, TUI owns it.

### Already mostly harness-sourced today

- Model and current effort: `get_state`, `set_model`, `set_effort` response paths.
- Context window and context/cost stats: `get_state` + `get_session_stats`.
- Streaming/running state: agent events and execution state.
- Workflow state and automation flags: `get_state` workflow object and workflow events.
- Conversation history/messages: `get_messages`, `get_message`, sync/ledger events.
- Subagent roster and liveness: `get_subagents` and `SubagentStateChanged`.
- Extension list: `get_extensions` / `ExtensionsChanged`.
- Session list and resume/new-session outcomes: harness commands.

### Should be moved or tightened

1. **Workspace cwd / display path**
   - Current source: TUI `std::env::current_dir()`.
   - Desired source: harness `SessionState.workspace.cwd`.

2. **Git branch / repository root**
   - Current source: TUI `.git/HEAD` polling.
   - Desired source: harness `SessionState.workspace.gitBranch` and optional `repoRoot`.

3. **Workspace file list / `@file` autocomplete**
   - Current source: TUI scans local cwd and runs local git commands.
   - Desired source: harness workspace path at minimum; ideally a harness command/event for file search/listing.

4. **Sandbox/read-only/tool availability presentation**
   - Current source: partly implied by TUI launch mode and tool events.
   - Desired source: harness capability snapshot: sandbox mode, read-only status, disabled tools, available native/extension tools. The TUI can then render badges or disable unsupported affordances without duplicating launch logic.

5. **Provider/model capability details**
   - Current source: model list and effort levels are harness-backed, but selector UX can still cache local assumptions.
   - Desired source: harness model registry response remains the single source for provider, model id, max context, effort levels, and pricing/cost availability.

6. **Session identity and attachment mode**
   - Current source: TUI tracks `connected_agent_id`, connection/child status, and child process watch for TUI-owned launches.
   - Desired source: harness should expose stable connected session identity, parent/child relationship, and lifecycle state; TUI-owned child process diagnostics can remain view-side because they describe the viewer's spawned process wrapper.

7. **Current workflow template library / available workflow actions**
   - Current source: active workflow state is harness-backed; template discovery/availability should also stay harness-backed because it depends on harness config and workspace.

8. **Auth/provider health**
   - Current source: errors/responses when commands fail.
   - Desired source: optional harness status/capabilities snapshot could let the TUI render provider/auth readiness without probing or interpreting config directly.

### Should remain TUI-owned

- Input draft text, cursor position, selections, paste buffering.
- Focus/viewport/scroll position and panel open/closed state.
- Rendering cache, terminal dimensions, theme, animations/spinners as presentation.
- Client-side notifications derived from responses, as long as underlying facts come from harness responses/events.
- TUI-owned child process exit diagnostics when the TUI itself spawned the harness process.

## Recommended sequencing

1. Add harness `workspace` fields to `SessionState` and wire both live and busy snapshot paths.
2. Parse and display those fields in TUI footer, preserving a compatibility fallback.
3. Remove TUI-side git polling once harness workspace updates are reliable.
4. Move file autocomplete to harness workspace path, then design a harness file-list/search command.
5. Audit remaining `std::env::current_dir()` and local git usage in `quecto-tui/src` and either remove it or classify it as presentation/test/fallback only.

## Open questions

- Should `cwd` mean the process cwd, resolved agent workspace root, or sandbox workspace? For user-facing and tool-relative semantics it should be the same root used by file tools.
- Should branch update arrive as part of periodic `get_state`, a pushed `WorkspaceStateChanged` event, or an update to the busy state snapshot? A pushed event is cleaner for TUI rendering, but a `get_state` field is the minimum contract.
- Can subagents have distinct workspaces? If yes, workspace metadata belongs per session/subagent, not only on the connected master.
- How much path information is safe to expose in remote/hosted harness modes? If absolute host paths are sensitive, include a display path plus a stable workspace id/root label.
