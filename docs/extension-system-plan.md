# Extension System & Tool Guard Plan

## Rationale

Quecto needs two things that are currently missing:

1. **Script extensions** — hot-reloadable tools from disk so the LLM or user can add new capabilities mid-session without restarting or recompiling.

2. **Tool guards** — a generic interception mechanism so workflow (and future concerns) can block tool calls at the wrong stage. Today workflow is a tool only — it can report state but cannot enforce it. The LLM can `git commit` at step 2 and there's nothing to stop it.

A secondary problem is **wiring scatter**. Adding the workflow tool required changes across 8 files in 4 layers. While workflow itself stays in core (it's fundamental to how Quecto keeps the LLM honest), the registration path should be cleaner.

### What This Plan Does

- Adds a `ToolGuard` trait so workflow can intercept and block `bash` calls like `git commit` and `git push` before they execute
- Adds a script extension system so new tools can be dropped onto disk and hot-reloaded
- Cleans up workflow wiring in the interface layer (consolidate, don't scatter)
- Core tools stay exactly where they are — no unnecessary moves

### What This Plan Does Not Do

- Move core tools (`bash`, `read`, `write`, `edit`, `ls`, `grep`, `find`) — they're core, they stay core
- Move workflow out of `domain/` — it's core vocabulary, it belongs there
- Dynamic linking / shared library plugins (ABI instability in Rust)
- WASM-based extensions (too heavy for RPi/container targets)
- Long-running sidecar extensions (fragile pipe/JSON comms, as proven by subagent experience)
- Extension dependency management or package registry

---

## Architecture After Migration

```
domain/
  tool.rs              ← Tool trait + ToolGuard trait (new, ~8 lines)
  extension.rs         ← Extension trait (new, ~15 lines)
  workflow.rs          ← stays, core vocabulary

infrastructure/
  tools/               ← stays, all core tools unchanged
    bash/
    filesystem/
    grep.rs
    find.rs
    spawn.rs
    cron_tool.rs
    message.rs
    web_search.rs
    recall.rs
    workflow_tool.rs   ← stays, gains WorkflowGuard
    registry.rs        ← gains guard mechanism
    path_utils.rs
    truncate.rs
    ensure_tool.rs

  extensions/          ← new
    mod.rs             ← public API
    registry.rs        ← ExtensionRegistry (discovery, loading, hot-reload)
    script.rs          ← ScriptExtension + ScriptTool (subprocess wrapper)
    watcher.rs         ← filesystem polling for hot-reload

  security/
    sandbox.rs         ← unchanged

interface/
  shared.rs            ← workflow wiring consolidated into single function
  cli/agent.rs         ← core tools + extensions + guard, clean registration
  gateway/mod.rs       ← same
```

### How Tool Execution Works After Migration

```
Agent loop calls: registry.execute("bash", arguments)
                          │
                          ▼
                  ┌───────────────┐
                  │  ToolGuard(s) │  ← WorkflowGuard checks: is git commit/push
                  │               │    allowed at this workflow step?
                  │  Ok(()) ──────┼──► proceed to tool
                  │  Err(reason) ─┼──► return ToolResult { is_error: true, content: reason }
                  └───────────────┘    (tool never executes)
                          │
                          ▼
                  ┌───────────────┐
                  │  Tool.execute │  ← bash/read/write/edit/script extension/etc.
                  └───────────────┘
```

Guards run synchronously before tool execution. If any guard returns `Err`, the tool is never spawned — no subprocess to kill, no partial execution to roll back.

---

## Phases

### Phase 1 — ToolGuard Trait

Add the `ToolGuard` trait to `domain/tool.rs` and wire it into `ToolRegistryImpl`.

**Changes:**
- Add `ToolGuard` trait to `domain/tool.rs` — `check(tool_name, arguments) -> Result<(), String>`
- Add `guards: Vec<Arc<dyn ToolGuard>>` to `ToolRegistryImpl`
- Add `register_guard()` method to `ToolRegistryImpl`
- Modify `ToolRegistryImpl::execute()` to run all guards before tool execution
- Guard rejection returns `ToolResult { is_error: true, content: reason }` — the agent loop sees an error result, not a `DomainError`

**Acceptance criteria:**
- [ ] `ToolGuard` trait exists in `domain/tool.rs` with `check(&self, tool_name: &str, arguments: &str) -> Result<(), String>`
- [ ] `ToolGuard` is `Send + Sync`
- [ ] `ToolRegistryImpl` holds `Vec<Arc<dyn ToolGuard>>`
- [ ] `register_guard(guard: Arc<dyn ToolGuard>)` adds a guard
- [ ] `execute()` runs all guards before tool execution
- [ ] Guards run in registration order — first `Err` short-circuits (remaining guards and tool are skipped)
- [ ] Guard rejection produces `ToolResult { content: reason, is_error: true, image_blocks: vec![] }`
- [ ] Guard rejection does NOT return `DomainError` — it's a normal error result the LLM sees and can act on
- [ ] When all guards return `Ok(())`, tool executes normally
- [ ] Empty guard list (default) has zero overhead — no allocation, no iteration
- [ ] `ToolRegistry` trait gains `register_guard()` or guards are `ToolRegistryImpl`-only (decide based on whether trait consumers need it)
- [ ] Unit tests: no guards → tool executes; one guard allows → executes; one guard blocks → error result with reason; multiple guards, first blocks → short-circuits; guard receives correct tool_name and arguments
- [ ] Existing tool tests pass unchanged
- [ ] `cargo test --lib` passes
- [ ] `cargo clippy -- -D warnings` passes

---

### Phase 2 — WorkflowGuard

Add `WorkflowGuard` to `infrastructure/tools/workflow_tool.rs` so workflow can block `git commit` and `git push` at the wrong stage.

**Changes:**
- Add `WorkflowGuard` struct to `workflow_tool.rs` — holds `Arc<Mutex<WorkflowState>>` and `enforce_commit_after_step: Option<u32>`
- Implement `ToolGuard` for `WorkflowGuard`
- Guard logic: parse bash command, detect `git commit`/`git push` patterns, check `WorkflowState::check_commit_allowed()`
- Non-bash tools pass through unchecked
- Update `interface/shared.rs` → consolidate workflow registration into a single `register_workflow()` that wires tool + guard + prompt snippet

**Command detection patterns:**
- `git commit` (with any flags/args)
- `git push` (with any flags/args)
- Handles: `git -c key=val commit`, `git -C /path commit`, pipes like `echo | git commit`
- Does NOT block: `git add`, `git status`, `git diff`, `git log`

**Acceptance criteria:**
- [ ] `WorkflowGuard` struct exists in `workflow_tool.rs`
- [ ] `WorkflowGuard` implements `ToolGuard`
- [ ] Non-bash tools (`read`, `write`, `edit`, `ls`, etc.) always pass — `Ok(())`
- [ ] Bash calls without `git commit`/`git push` always pass — `Ok(())`
- [ ] `git commit` blocked when `check_commit_allowed()` returns `Err` — guard returns `Err` with actionable message
- [ ] `git push` blocked under same conditions
- [ ] Error message tells the LLM: what was blocked, which step to complete, and to run `workflow(action='status')`
- [ ] `git commit` allowed when required steps are complete
- [ ] `git push` allowed when required steps are complete
- [ ] When `enforce_commit_after_step` is `None`, all commits/pushes pass
- [ ] Guard handles edge cases: `git -c user.name=x commit`, `git -C /tmp commit`, multi-line bash scripts containing `git commit`
- [ ] Guard does NOT false-positive on: `git add --all && echo "committed"`, `echo "git commit"` (string literal in echo), `# git commit` (comments)
- [ ] `interface/shared.rs` has a single `register_workflow()` function that registers tool + guard + prompt snippet
- [ ] `interface/cli/agent.rs` calls `register_workflow()` once — no other workflow references
- [ ] `interface/gateway/mod.rs` calls `register_workflow()` once — no other workflow references
- [ ] `append_workflow_prompt()` is removed from `interface/shared.rs` (folded into `register_workflow()`)
- [ ] `register_workflow_tool()` is removed from `interface/shared.rs` (folded into `register_workflow()`)
- [ ] Workflow unit tests all pass
- [ ] New guard-specific tests cover all detection patterns
- [ ] `cargo test --lib` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] BDD tests pass (all 24 shards)

---

### Phase 3 — Extension Trait

Add `domain/extension.rs` with the `Extension` trait. Pure types for the script extension system.

**Changes:**
- Create `domain/extension.rs` — `Extension` trait with `name()`, `tools()`, `system_prompt_snippet()`
- Add `pub mod extension` to `domain/mod.rs`

**Acceptance criteria:**
- [ ] `domain/extension.rs` exists with `Extension` trait
- [ ] `Extension` trait has methods: `name() -> &str`, `tools() -> Vec<Arc<dyn Tool>>`, `system_prompt_snippet() -> Option<String>`
- [ ] `system_prompt_snippet()` has a default implementation returning `None`
- [ ] Trait is `Send + Sync` and dyn-compatible
- [ ] No other files modified beyond `domain/mod.rs` adding the module
- [ ] `cargo test --lib` passes
- [ ] `cargo clippy -- -D warnings` passes

---

### Phase 4 — Extension Registry

Create `infrastructure/extensions/` with the registry that discovers script extensions from disk and produces tools + prompt snippets.

**Changes:**
- Create `infrastructure/extensions/mod.rs`
- Create `infrastructure/extensions/registry.rs` — `ExtensionRegistry` with `discover()`, `all_tools()`, `system_prompt_snippets()`
- Wire into `infrastructure/mod.rs`

**Acceptance criteria:**
- [ ] `ExtensionRegistry::new()` creates an empty registry
- [ ] `register(ext: Arc<dyn Extension>)` adds an extension
- [ ] `all_tools()` returns tools from all registered extensions, deduplicated by name (last wins)
- [ ] `system_prompt_snippets()` returns concatenated non-empty snippets from all extensions
- [ ] Extensions with the same tool name: later registration overrides earlier (project-local overrides global)
- [ ] `discover(dirs: &[PathBuf])` scans directories and registers discovered extensions
- [ ] Unit tests cover: empty registry, single extension, multiple extensions, tool deduplication, prompt concatenation
- [ ] `cargo test --lib` passes
- [ ] `cargo clippy -- -D warnings` passes

---

### Phase 5 — Script Extension Loading

Add the ability to load extensions from `extension.toml` manifests on disk and execute them as subprocesses.

**Changes:**
- Create `infrastructure/extensions/script.rs` — `ExtensionManifest` (TOML deserialization), `ScriptExtension`, `ScriptTool`
- `ScriptTool` implements `Tool`: spawns subprocess, pipes JSON args on stdin, reads JSON result from stdout
- `discover_script_extensions(dir: &Path)` scans a directory for `*/extension.toml` and returns `Vec<Arc<dyn Extension>>`

**Manifest format:**
```toml
name = "tool_name"
description = "What this tool does.\nExample: {\"arg\": \"value\"}"
parameters_schema = '{"type":"object","properties":{"arg":{"type":"string"}},"required":["arg"]}'
command = "./script.sh"
timeout_secs = 30        # optional, default 30
system_prompt = "..."    # optional
```

**Script protocol:**
- Stdin: raw JSON arguments string
- Stdout: single JSON object `{"content": "...", "is_error": false}`
- Non-zero exit code: `is_error: true`, stderr captured as content
- Timeout: `tokio::time::timeout` kills the child process

**Discovery locations** (in priority order, later overrides earlier):
```
~/.config/quecto/extensions/*/extension.toml     # global user extensions
<workspace>/.quecto/extensions/*/extension.toml  # project-local (override/add)
```

**Acceptance criteria:**
- [ ] `ExtensionManifest` deserialises from TOML with all fields (name, description, parameters_schema, command, timeout_secs, system_prompt)
- [ ] `timeout_secs` defaults to 30 when omitted
- [ ] `system_prompt` is `Option<String>`, returned by `Extension::system_prompt_snippet()`
- [ ] `ScriptTool::definition()` returns correct `ToolDefinition` from manifest
- [ ] `ScriptTool::execute()` spawns subprocess, pipes args on stdin, parses JSON stdout
- [ ] Non-zero exit code produces `ToolResult { is_error: true, content: stderr }`
- [ ] Empty stderr on non-zero exit produces `ToolResult { is_error: true, content: "process exited with code N" }`
- [ ] Timeout kills the child process and returns `ToolResult { is_error: true, content: "extension 'name' timed out after Ns" }`
- [ ] Invalid stdout JSON returns `ToolResult { is_error: true, content: "invalid output from extension 'name': ..." }`
- [ ] `discover_script_extensions()` finds all `*/extension.toml` in a directory
- [ ] `discover_script_extensions()` skips directories without `extension.toml`
- [ ] `discover_script_extensions()` skips manifests with invalid TOML (logs warning, continues)
- [ ] `discover_script_extensions()` returns empty vec for non-existent directory
- [ ] `command` is resolved relative to the manifest directory
- [ ] `command` that is not executable returns `ToolResult { is_error: true }` with descriptive message
- [ ] Tool guards apply to script extension tools (they're registered in the same `ToolRegistryImpl`)
- [ ] Integration test: real script extension discovered, loaded, and executed successfully
- [ ] `cargo test --lib` passes
- [ ] `cargo clippy -- -D warnings` passes

---

### Phase 6 — Hot-Reload Watcher

Add filesystem polling that detects changes in extension directories and triggers reload.

**Changes:**
- Create `infrastructure/extensions/watcher.rs` — `spawn_watcher()` function
- Add `reload_scripts()` to `ExtensionRegistry` — re-scans disk, replaces script extensions
- Add `fingerprint_dirs()` — returns `HashMap<PathBuf, (SystemTime, u64)>` of all `extension.toml` files
- When extensions change, the `ToolRegistryImpl` is updated: old script tools removed, new script tools registered
- Thread safety: `ExtensionRegistry` uses `RwLock` internally so reload is safe between agent turns

**Acceptance criteria:**
- [ ] `fingerprint_dirs()` returns mtime + size for every `extension.toml` in watched directories
- [ ] `fingerprint_dirs()` handles non-existent directories gracefully (empty map)
- [ ] `reload_scripts()` removes previously registered script extension tools from the tool registry
- [ ] `reload_scripts()` registers newly discovered script extension tools
- [ ] `reload_scripts()` handles new extensions (added since last scan)
- [ ] `reload_scripts()` handles removed extensions (directory deleted since last scan)
- [ ] `reload_scripts()` handles modified extensions (manifest changed since last scan)
- [ ] `reload_scripts()` does not affect core tools or guards
- [ ] `spawn_watcher()` returns a `JoinHandle` that polls at the configured interval
- [ ] `spawn_watcher()` calls `reload_scripts()` only when fingerprint changes
- [ ] `spawn_watcher()` logs on reload (added/removed/total extension count)
- [ ] No new dependencies — uses `tokio::fs::metadata` and `tokio::time::sleep`
- [ ] Default poll interval: 5 seconds, configurable
- [ ] Unit tests cover: fingerprint change detection, reload adds new extension, reload removes deleted extension, reload updates modified extension, core tools unaffected by reload
- [ ] `cargo test --lib` passes
- [ ] `cargo clippy -- -D warnings` passes

---

### Phase 7 — Interface Integration

Wire the extension registry into the CLI agent and gateway entry points alongside the existing core tool registration.

**Changes:**
- `interface/cli/agent.rs`: after core tool registration, discover and register script extensions
- `interface/gateway/mod.rs`: same
- Start the hot-reload watcher in long-running modes (REPL, gateway, UDS agent)
- Do NOT start the watcher in one-shot mode (`quecto agent -m "..."`)

**Acceptance criteria:**
- [ ] `interface/cli/agent.rs` registers core tools via `ToolRegistryImpl::with_core_tools_and_exec_settings()` (unchanged)
- [ ] `interface/cli/agent.rs` then registers script extension tools from `ExtensionRegistry::discover()`
- [ ] `interface/cli/agent.rs` appends extension system prompt snippets to the system prompt
- [ ] `interface/gateway/mod.rs` does the same
- [ ] Hot-reload watcher is started in REPL mode
- [ ] Hot-reload watcher is started in gateway mode
- [ ] Hot-reload watcher is started in UDS agent mode (`--mode uds`)
- [ ] Hot-reload watcher is NOT started in one-shot mode (`quecto agent -m`)
- [ ] Script extension tools appear in tool definitions sent to the LLM
- [ ] Tool guards apply to script extension tools
- [ ] All entry points produce the same core tool set as before (script extensions are additive)
- [ ] `cargo test --lib` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] BDD tests pass (all 24 shards)
- [ ] Architecture tests pass (`cargo test --test architecture`)

---

### Phase 8 — Documentation and Examples

Provide documentation and reference implementations for extension authors.

**Changes:**
- Create `docs/extensions.md` — user-facing guide
- Create `examples/extensions/hello/` — minimal bash script extension
- Create `examples/extensions/python-tool/` — Python script extension with error handling and system_prompt
- Update `AGENTS.md` — document extension system, tool guard mechanism, script protocol

**Acceptance criteria:**
- [ ] `docs/extensions.md` covers: manifest format (`extension.toml`), script protocol (stdin JSON, stdout JSON, stderr, exit codes), discovery locations (global `~/.config/quecto/extensions/` + project-local `<workspace>/.quecto/extensions/`), override precedence (project-local wins), hot-reload behaviour and timing (5s default poll), timeout handling, error handling
- [ ] `docs/extensions.md` documents the `system_prompt` manifest field and how it's injected
- [ ] `docs/extensions.md` includes security note: script extensions run with user's permissions
- [ ] `examples/extensions/hello/extension.toml` + `hello.sh` is a working extension that can be discovered and executed
- [ ] `examples/extensions/python-tool/extension.toml` + `tool.py` is a working extension demonstrating error handling, timeout, and system_prompt
- [ ] Both example extensions pass manual smoke test: discovered, loaded, executed, hot-reloaded after modification
- [ ] `AGENTS.md` documents: `ToolGuard` trait and its role, `WorkflowGuard` behaviour (which commands are blocked and when), extension system architecture, script extension locations and protocol
- [ ] `AGENTS.md` tool table updated to include script extensions as a concept
- [ ] `cargo test --lib` passes
- [ ] `cargo clippy -- -D warnings` passes

---

## What Changes vs What Stays

### Added (new files)

| File | Purpose |
|---|---|
| `domain/tool.rs` | `ToolGuard` trait (~8 lines added) |
| `domain/extension.rs` | `Extension` trait (~15 lines) |
| `infrastructure/extensions/mod.rs` | Module root |
| `infrastructure/extensions/registry.rs` | `ExtensionRegistry` — discovery, loading, hot-reload coordination |
| `infrastructure/extensions/script.rs` | `ScriptExtension`, `ScriptTool`, `ExtensionManifest` |
| `infrastructure/extensions/watcher.rs` | `spawn_watcher()`, `fingerprint_dirs()` |
| `infrastructure/tools/workflow_tool.rs` | `WorkflowGuard` (~40 lines added) |
| `docs/extensions.md` | User-facing extension guide |
| `examples/extensions/` | Reference implementations |

### Modified (existing files, minimal changes)

| File | Change |
|---|---|
| `domain/mod.rs` | Add `pub mod extension` |
| `infrastructure/mod.rs` | Add `pub mod extensions` |
| `infrastructure/tools/registry.rs` | Add `guards` field, `register_guard()`, guard loop in `execute()` |
| `interface/shared.rs` | Consolidate `append_workflow_prompt()` + `register_workflow_tool()` → single `register_workflow()` |
| `interface/cli/agent.rs` | Add extension discovery + registration after core tools; start watcher |
| `interface/gateway/mod.rs` | Same |

### Not Moved, Not Deleted

| File | Why It Stays |
|---|---|
| `domain/workflow.rs` | Core vocabulary — workflow state is fundamental to Quecto's operation |
| `domain/workflow_tests.rs` | Tests for core domain types |
| `infrastructure/tools/workflow_tool.rs` | Core tool — workflow needs in-process state and guard capability |
| `infrastructure/tools/registry.rs` | Core registry — gains guards, otherwise unchanged |
| `infrastructure/tools/bash/` | Core tool |
| `infrastructure/tools/filesystem/` | Core tools |
| `infrastructure/tools/grep.rs` | Core tool |
| `infrastructure/tools/find.rs` | Core tool |
| `infrastructure/tools/spawn.rs` | Core tool |
| All other `infrastructure/tools/*` | Core tools — no reason to move working code |

---

## Script Extension Details

### Manifest Format — `extension.toml`

```toml
name        = "csv_parser"
description = """
Parse a CSV file and return structured data.
Example: {"path": "data.csv", "delimiter": ","}
"""

parameters_schema = """
{
  "type": "object",
  "properties": {
    "path": { "type": "string", "description": "Path to CSV file" },
    "delimiter": { "type": "string", "description": "Column delimiter", "default": "," }
  },
  "required": ["path"]
}
"""

command = "./parse.py"
timeout_secs = 60

system_prompt = "When the user asks about CSV data, use the csv_parser tool."
```

### Script Protocol

**Input:** JSON arguments on stdin (the raw string from the LLM).

**Output:** Single JSON object on stdout:

```json
{"content": "Parsed 42 rows with 5 columns.", "is_error": false}
```

**Errors:**
- Non-zero exit → `is_error: true`, stderr as content
- Invalid JSON on stdout → `is_error: true`, raw output in error message
- Timeout → process killed, `is_error: true`

**Any language works:**

```bash
#!/usr/bin/env bash
# hello.sh
input=$(cat)
name=$(echo "$input" | jq -r '.name')
echo "{\"content\": \"Hello, $name!\", \"is_error\": false}"
```

```python
#!/usr/bin/env python3
# tool.py
import json, sys
args = json.load(sys.stdin)
json.dump({"content": f"Processed: {args['path']}", "is_error": False}, sys.stdout)
```

### Discovery

```
~/.config/quecto/extensions/
  hello/
    extension.toml
    hello.sh

  csv-parser/
    extension.toml
    parse.py

<workspace>/.quecto/extensions/
  project-lint/
    extension.toml
    lint.sh
```

Global extensions load first. Project-local extensions with the same `name` override the global one.

### Hot-Reload Behaviour

- Watcher polls every 5 seconds (configurable)
- Compares mtime + size of all `extension.toml` files
- On change: removes old script tools from registry, discovers and registers new ones
- Core tools and guards are never affected
- New tools appear in the LLM's next turn
- Removed tools disappear from the LLM's next turn
- In-flight tool executions are not interrupted (held via `Arc`)

### What the LLM Can Do Mid-Session

1. LLM uses `write` to create `~/.config/quecto/extensions/mytool/extension.toml`
2. LLM uses `write` to create `~/.config/quecto/extensions/mytool/tool.sh`
3. LLM uses `bash` to run `chmod +x ~/.config/quecto/extensions/mytool/tool.sh`
4. Watcher detects the new files within 5 seconds
5. `mytool` appears in tool definitions on the LLM's next turn
6. LLM calls `mytool` — subprocess spawns, JSON in/out, result returned

The LLM just extended its own capabilities without a restart.

---

## Tool Guard Details

### How WorkflowGuard Works

```
LLM calls: bash({"command": "git commit -m 'implement feature'"})

→ ToolRegistryImpl::execute("bash", arguments)
  → WorkflowGuard::check("bash", arguments)
    → parses command: detects "git commit"
    → checks WorkflowState::check_commit_allowed(enforce_commit_after_step)
    → step 6 not complete
    → returns Err("BLOCKED: cannot git commit — complete step 6 (Ensure tests pass)
       first. Run workflow(action='status') to see current progress.")

→ LLM receives error result
→ LLM checks workflow status, completes remaining steps
→ LLM tries git commit again
→ WorkflowGuard allows it
→ bash executes normally
```

### Detection Patterns

The guard detects these patterns in bash commands:

| Pattern | Blocked? | Why |
|---|---|---|
| `git commit -m "msg"` | ✅ | Direct commit |
| `git push origin main` | ✅ | Direct push |
| `git -c user.name=x commit` | ✅ | Commit with config override |
| `git -C /path commit` | ✅ | Commit in different directory |
| `git add . && git commit` | ✅ | Chained commit |
| `git add .` | ❌ | Staging is always allowed |
| `git status` | ❌ | Read-only |
| `git diff` | ❌ | Read-only |
| `git log` | ❌ | Read-only |
| `echo "git commit"` | ❌ | String literal, not a command |
| `# git commit` | ❌ | Comment |

### Future Guards

The `ToolGuard` mechanism is generic. Future uses:

- **SecurityGuard** — block `rm -rf /`, `curl | bash`, `chmod 777` patterns
- **SandboxGuard** — enforce workspace boundaries at the command level (complementing filesystem sandbox)
- **RateLimitGuard** — throttle expensive tools (web search, LLM-calling tools)
- **AuditGuard** — log all tool calls for compliance

Each is an `Arc<dyn ToolGuard>` registered on the same registry. They compose — all guards must pass for a tool to execute.

---

## Migration Risk Assessment

| Risk | Mitigation |
|---|---|
| Guard false positives blocking legitimate git commands | Comprehensive pattern tests. Guard only checks for `commit` and `push` subcommands. `git add`, `git status`, `git diff`, `git log` etc. always pass. |
| Guard false negatives (LLM finds workaround) | Defence in depth — guard is one layer. LLM can always `bash -c "$(echo Z2l0IGNvbW1pdA== \| base64 -d)"`. Accept this limitation; the guard catches honest mistakes, not adversarial attacks. |
| Script extension subprocess overhead | ~2-5ms spawn per call. Acceptable for dev tooling. Core tools have zero overhead change. |
| Hot-reload race with agent loop | `RwLock` on extension list. Reload between turns is safe. Mid-tool-execution reload doesn't affect running tool (Arc reference held). |
| Script extension security | Script extensions run with user's permissions (same as bash tool). Project-local extensions are user-created. Document the trust model. |
| Polling overhead on RPi | 5-second interval, `stat()` on ~10 files. Negligible. Watcher can be disabled via config. |
| TOML parsing adds dependency | `toml` crate is lightweight (~30KB). Only alternative is YAML (already present via `serde_yaml`) but TOML is more natural for config manifests. Could use YAML instead to avoid the new dep — decide during implementation. |

## Phase Dependencies

```
Phase 1 (ToolGuard trait)
  └─► Phase 2 (WorkflowGuard)

Phase 3 (Extension trait)
  └─► Phase 4 (Extension registry)
        └─► Phase 5 (Script extension loading)
              └─► Phase 6 (Hot-reload watcher)

Phase 2 + Phase 6
  └─► Phase 7 (Interface integration)
        └─► Phase 8 (Documentation)
```

The two tracks (guards and extensions) are independent until Phase 7. They can be developed in parallel.

```
Track A: Phase 1 → Phase 2 ─────────────────────┐
                                                  ├─► Phase 7 → Phase 8
Track B: Phase 3 → Phase 4 → Phase 5 → Phase 6 ─┘
```

## Estimated Effort

| Phase | Scope | Estimate |
|---|---|---|
| 1 — ToolGuard trait | ~8 lines trait, ~20 lines registry change, tests | Small |
| 2 — WorkflowGuard | ~40 lines guard, ~30 lines detection, consolidate shared.rs, tests | Medium |
| 3 — Extension trait | 1 new file, ~15 lines, 1 line mod change | Small |
| 4 — Extension registry | 1 new file, discovery logic, tests | Small |
| 5 — Script extensions | Manifest parsing, subprocess execution, integration tests | Medium |
| 6 — Hot-reload watcher | Polling, fingerprinting, reload coordination, tests | Small |
| 7 — Interface integration | Wire into agent.rs + gateway, start watcher | Medium |
| 8 — Documentation | docs/extensions.md, examples, AGENTS.md update | Small |
