# Coding Tools: pi-mono vs. quecto

> Research date: 2026-02-27  
> pi-mono path: `/home/swq/Documents/github/pi-mono/packages/coding-agent/src/core/tools/`  
> quecto path: `src/infrastructure/tools/`

---

## Overview

| Dimension | pi-mono (TypeScript) | quecto (Rust) |
|---|---|---|
| Language | TypeScript | Rust |
| Tool interface | `AgentTool<Schema>` — generic, typed via TypeBox JSON Schema | `dyn Tool` trait with `Pin<Box<dyn Future>>` for dyn-safety |
| Schema definition | TypeBox (`Type.Object(...)`) at compile time | Raw JSON Schema strings embedded in `ToolDefinition` |
| Output type | Rich `{ content: Content[], details: T }` with structured detail objects | Simple `ToolResult { content: String, is_error: bool }` |
| Pluggability | Each tool has a `createXxxTool(cwd, options)` factory with swappable `XxxOperations` interface (e.g. for SSH remoting) | Tools take `Arc<PathBuf>` + `Arc<Sandbox>` — not designed for remote swapping |
| Security | Relies on `cwd` scoping; no sandbox abstraction | Explicit `Sandbox` struct enforcing path + command allowlisting |
| Registry | `allTools` record / `createAllTools()` helper | `ToolRegistryImpl` — a `HashMap<name, Arc<dyn Tool>>` |

---

## Tool Sets

### pi-mono

| Set | Tools |
|---|---|
| `codingTools` (default) | `read`, `bash`, `edit`, `write` |
| `readOnlyTools` | `read`, `grep`, `find`, `ls` |
| `allTools` | All 7 |

### quecto

| Set | Tools |
|---|---|
| Core registry (`with_core_tools`) | `exec`, `read_file`, `write_file`, `edit_file`, `append_file`, `list_dir` |
| Extended (registered ad-hoc) | `spawn`, `web_search`, `message`, `cron`, `recall`, `coding_delegation`, `coding_job` |
| Worker helpers (not LLM-facing) | `grep_content`, `find_files`, `read_file_paginated`, `edit_file` (worker_tools.rs) |

---

## Tool-by-Tool Comparison

### 📖 Read File

| | pi-mono `read` | quecto `ReadFileTool` / `read_file_paginated` |
|---|---|---|
| Parameters | `path`, optional `offset` (1-indexed), optional `limit` | `path` only (in `ReadFileTool`); `offset` + `limit` in `read_file_paginated` worker helper |
| Image support | ✅ jpg/png/gif/webp — sent as base64 with optional auto-resize to 2000×2000 | ❌ None |
| Truncation | Head-truncation at 2000 lines / 50 KB; actionable "use offset=N" continuation messages | Hard 1 MB file size cap — rejects oversized files outright |
| Pagination | Built into the main LLM-facing tool via `offset`/`limit` | Pagination exists only in `read_file_paginated()`, a worker-internal helper |
| Security | `cwd`-relative path resolution | `Sandbox.validate_path()` blocks traversal outside workspace |

---

### ✏️ Edit File

| | pi-mono `edit` | quecto `EditFileTool` (filesystem.rs) | quecto `edit_file` (worker_tools.rs) |
|---|---|---|---|
| Match algorithm | Exact first, then fuzzy (whitespace-normalised) | Plain `str::contains()` + `str::replacen()` | Exact → fuzzy trailing-whitespace → smart punctuation fallback |
| Uniqueness guard | ✅ Rejects multiple occurrences | ❌ Replaces only first match silently | ✅ Rejects ambiguous multi-matches, reports line numbers |
| Diff output | ✅ Unified diff in `details.diff` + first changed line | ❌ None | ✅ Unified diff + first changed line |
| BOM / CRLF | ✅ Strips BOM, preserves original line endings | ❌ None | ✅ Full BOM + CRLF detection and preservation |
| Preview mode | ❌ No | ❌ No | ✅ `preview_only` flag computes diff without writing |
| No-op guard | ✅ Errors if replacement produces identical content | ❌ No | ✅ Errors on identical `old`/`new` strings |

**Note:** quecto has two separate edit implementations. The LLM-facing `EditFileTool` in the core registry is minimal. The richer `worker_tools::edit_file` is only used inside the coding worker sandbox.

---

### 📝 Write File

| | pi-mono `write` | quecto `WriteFileTool` |
|---|---|---|
| Auto-create parent dirs | ✅ Yes | ✅ Yes |
| Security | `cwd`-relative | Sandbox path validation |
| Notes | Functionally equivalent; quecto adds sandbox enforcement |

---

### ➕ Append File

| | pi-mono | quecto `AppendFileTool` |
|---|---|---|
| Exists | ❌ No equivalent | ✅ Appends bytes to a file, creates if absent |

pi-mono has no append primitive — the agent must `read` + `write` to simulate it.

---

### 🔧 Shell Execution

| | pi-mono `bash` | quecto `ExecTool` |
|---|---|---|
| Shell | `sh -c` (configurable via `spawnHook`) | `sh -c` (native) or full **nsjail** namespace jail |
| Sandbox isolation | ❌ None beyond `cwd` scoping | ✅ Two-tier: native or **nsjail** with namespace/cgroup/seccomp; configurable memory/PID/CPU limits |
| Output handling | Streaming with rolling buffer; tail-truncated to 2000 lines / 50 KB; full output spilled to a temp file with path returned | Captured up to `max_capture_bytes` (default 1 MB); stdout and stderr kept separate |
| Timeout | Optional per-call `timeout` parameter | Fixed at construction (`DEFAULT_EXEC_TIMEOUT = 30 s`); not per-call |
| Abort / cancellation | ✅ `AbortSignal` support with full process-tree kill | ❌ No abort signal support |
| Environment filtering | Inherits full shell env; `spawnHook` can override any part | Strict allowlist (`HOME`, `PATH`, `LANG`, `TZ`, `TERM`, `SHELL`, `USER`, `LOGNAME`, `TMPDIR`, `LC_*`); `QUECTO_*` vars always blocked |
| Spawn hook | ✅ `BashSpawnHook` for command / cwd / env customisation | ❌ None |
| nsjail features | — | Binary path validation (trusted dirs only), `--die_with_parent`, cgroup v2 probe, fallback to rlimits when cgroups unavailable |

---

### 🔍 Grep / Search

| | pi-mono `grep` | quecto `grep_content` (worker helper) |
|---|---|---|
| Backend | **ripgrep** (`rg`) — auto-downloaded if missing | Pure Rust `fs::read_to_string` + literal substring match |
| Regex | ✅ Full regex | ❌ Literal only |
| Gitignore | ✅ | ✅ |
| Context lines | ✅ `context` param (lines before/after match) | ❌ None |
| LLM-facing tool | ✅ Yes | ❌ Worker internal only |
| Output limit | 100 matches / 50 KB | 1000 matches hard cap |

---

### 🗂️ Find Files

| | pi-mono `find` | quecto `find_files` (worker helper) |
|---|---|---|
| Backend | **fd** — auto-downloaded if missing | Pure Rust `fs::read_dir` + custom glob engine (`*`, `**`, `?`) |
| Gitignore | ✅ | ✅ |
| LLM-facing tool | ✅ Yes | ❌ Worker internal only |
| Output limit | 1000 results / 50 KB | No cap |

---

### 📂 List Directory

| | pi-mono `ls` | quecto `ListDirTool` |
|---|---|---|
| Alphabetical sort | ✅ Case-insensitive | ✅ |
| Directory suffix | ✅ `/` appended | ✅ `/` appended |
| Dotfiles | ✅ Included | ✅ Included |
| Output limit | 500 entries / 50 KB | None |
| Gitignore | ❌ | ❌ |

---

## Tools Unique to quecto

These have no equivalent in pi-mono's coding tool set.

### 🌐 `web_search`
Brave Search API with automatic DuckDuckGo fallback when no API key is configured. Returns top 5 results with title, URL, and description. Configurable base URLs for test mocking.

### 📨 `message`
Sends text to the user over their configured channel (e.g. Telegram) via the async message bus (`mpsc::Sender<OutboundMessage>`). Supports an explicit `target` override or falls back to the conversation's default target.

### ⏰ `cron`
Full cron job manager: `add`, `remove`, `list`, `enable`, `disable` actions. Supports interval-based (`interval_seconds`) and standard 5-field cron expression schedules. Validates `deliver_to` targets and cron expressions at add-time.

### 👶 `spawn`
Spawns a child `quecto agent` subprocess for background tasks. Validates `agent_id` format (`[a-zA-Z0-9_-]`, 1–64 chars) against an allowlist. 120 s timeout; inherits workspace restriction settings.

### 🔁 `recall`
Retrieves previously "spilled" (context-collapsed) tool outputs by ID. Enables the agent to re-expand outputs that were truncated out of the context window. Also supports `recall("list")` to get a full index of spilled entries. Tracks repeated recall counts and warns on recall-collapse loops.

---

## Key Architectural Differences

### 1. Security Posture
quecto is fundamentally more security-conscious. Every filesystem tool passes paths through `Sandbox.validate_path()` before touching disk, command execution has an env allowlist that strips secrets, and `ExecTool` optionally runs inside an nsjail namespace jail with cgroup-based resource limits.

pi-mono trusts `cwd`-relative path resolution entirely and has no sandbox abstraction.

### 2. Edit Quality Gap
pi-mono's `edit` and quecto's `worker_tools::edit_file` are both high-quality (fuzzy matching, BOM, CRLF, diff output). However the `EditFileTool` registered in quecto's core `ToolRegistryImpl` is much simpler — a plain `contains()` + `replacen()` with no diff, no uniqueness guard beyond first-match semantics, and no BOM/CRLF handling. This is a capability gap for the main agent.

### 3. Output Richness
pi-mono returns structured `details` objects alongside content — diffs, truncation metadata, first changed line numbers. These are consumed by the TUI to render inline diffs and navigate to changed lines. quecto uses a flat `{ content: String, is_error: bool }` — richer worker_tools data is serialised to strings before the LLM sees it.

### 4. Streaming vs. Buffering
pi-mono streams bash output incrementally via `onUpdate` callbacks, allowing the TUI to show live progress. quecto buffers all exec output until the process exits.

### 5. Search Tool Availability
quecto's `grep_content` and `find_files` are worker-internal helpers — they are **not** registered as LLM-callable tools in the main agent registry. The main agent has no grep or find capability. pi-mono exposes both as first-class LLM tools.

### 6. Tool Surface
quecto covers the full agent lifecycle (messaging, cron scheduling, context recall, subagent spawning, web search) — all absent from pi-mono's focused coding tool set. pi-mono is intentionally narrow; quecto is a complete personal assistant runtime.

### 7. Pluggability
pi-mono's `createXxxTool(cwd, options)` factory pattern with swappable `XxxOperations` interfaces is designed for remote execution (e.g. SSH). quecto's tools are wired to the local filesystem and are not designed for remote delegation.
