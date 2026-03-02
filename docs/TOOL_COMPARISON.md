# Pi vs Quecto — Tool-by-Tool Comparison

> Generated 2026-02-28. Pi source: `@mariozechner/pi-coding-agent` (dist/core/tools). Quecto source: `src/infrastructure/tools/`.

---

## 1. Pi's Built-in Tools (7 tools)

Pi ships four **default** tools (`read`, `bash`, `edit`, `write`) and three **optional** tools (`grep`, `find`, `ls`). Tools are selectable via `--tools read,bash,edit,write,grep,find,ls`.

### 1.1 `read` — Read file contents

| Aspect | Detail |
|--------|--------|
| **Parameters** | `path` (string, required), `offset` (number, optional, 1-indexed), `limit` (number, optional) |
| **Description** | Reads text files and images (jpg/png/gif/webp). Text output truncated to **2 000 lines** or **50 KB** (whichever first). |
| **Key behaviours** | • Resolves paths relative to cwd; supports `~` expansion, `@` prefix stripping, absolute paths. <br>• **macOS filename fixups**: tries NFD normalization, narrow-no-break-space AM/PM, curly-quote variants. <br>• **Image support**: detects MIME from file, reads as base64, auto-resizes, returns `{type:"image", data, mimeType}` content blocks. <br>• **Offset/limit pagination**: 1-indexed offset, user limit or auto truncation. Tells model the next offset to continue. <br>• **First-line-exceeds-limit guard**: if a single line > 50 KB, returns a `sed` hint instead of the line. <br>• **BOM-aware**: not explicitly mentioned in read but the edit tool strips BOM; read passes raw text. <br>• **Abort signal**: full abort/cancel support via AbortSignal. |
| **Output** | Array of content blocks (`text` or `image`). Includes truncation notices with actionable `offset=` hints. |

### 1.2 `write` — Write file contents

| Aspect | Detail |
|--------|--------|
| **Parameters** | `path` (string, required), `content` (string, required) |
| **Description** | Creates or overwrites a file. Auto-creates parent directories. |
| **Key behaviours** | • `mkdir -p` on parent dir before writing. <br>• Writes UTF-8. <br>• Returns byte count on success: `"Successfully wrote N bytes to path"`. <br>• Abort signal support. |

### 1.3 `edit` — Surgical find-and-replace

| Aspect | Detail |
|--------|--------|
| **Parameters** | `path` (string, required), `oldText` (string, required), `newText` (string, required) |
| **Description** | Replaces exact text in a file. oldText must match exactly (including whitespace). |
| **Key behaviours** | • **BOM stripping**: strips UTF-8 BOM before matching, re-prepends after writing. <br>• **Line-ending normalization**: detects original line endings (CRLF/LF), normalizes to LF for matching, restores original endings on write. <br>• **Fuzzy matching** (if exact match fails): strips trailing whitespace per line, normalizes smart quotes (`''""` → `'"`), normalizes Unicode dashes/hyphens → ASCII `-`, normalizes special Unicode spaces → regular space. <br>• **Uniqueness enforcement**: if the fuzzy-normalized text appears more than once, rejects with an error asking for more context. <br>• **No-op detection**: if replacement produces identical content, rejects with an error. <br>• **Diff generation**: produces a unified diff with line numbers and 4 lines of context for TUI display. <br>• Abort signal support. |
| **Output** | Success message + `details.diff` (unified diff string) + `details.firstChangedLine`. |

### 1.4 `bash` — Shell execution

| Aspect | Detail |
|--------|--------|
| **Parameters** | `command` (string, required), `timeout` (number, optional, in seconds) |
| **Description** | Executes a bash command in cwd. Output truncated to last **2 000 lines** or **50 KB**. |
| **Key behaviours** | • Spawns `sh -c` (or configurable shell) with `detached: true` in cwd. <br>• **Streaming output**: merges stdout+stderr into a single stream, streams via `onUpdate` callback for live TUI updates. <br>• **Tail truncation**: keeps the **last** 2000 lines / 50 KB (unlike read which keeps the **first**). Rationale: errors/results are at the end. <br>• **Temp file for large output**: if output > 50 KB, writes full output to `/tmp/pi-bash-*.log` and includes path in truncation notice. <br>• **Rolling buffer**: keeps a 100 KB rolling buffer of chunks for efficient tail-truncation of streaming output. <br>• **Timeout**: optional, kills entire process tree on timeout via `killProcessTree(pid)`. <br>• **Abort signal**: kills process tree on abort. <br>• **Non-zero exit**: returns error with exit code appended. <br>• **Command prefix hook**: optional prefix (e.g., `shopt -s expand_aliases`). <br>• **Spawn hook**: optional transform of `{command, cwd, env}` before spawning. <br>• **Shell env**: uses `getShellEnv()` for environment. <br>• **No stdin**: stdin is `"ignore"`. |
| **Output** | Text content with truncation notices pointing to temp file. Non-zero exit codes surface as errors. |

### 1.5 `grep` — Ripgrep-powered search

| Aspect | Detail |
|--------|--------|
| **Parameters** | `pattern` (string, required), `path` (optional, default `.`), `glob` (optional), `ignoreCase` (optional bool), `literal` (optional bool), `context` (optional number), `limit` (optional, default 100) |
| **Description** | Searches file contents with ripgrep. Respects .gitignore. |
| **Key behaviours** | • **Auto-downloads ripgrep** via `ensureTool("rg")` if not available. <br>• Runs `rg --json --line-number --color=never --hidden` with optional flags. <br>• **Match limit**: default 100, kills rg process when limit reached. <br>• **Context lines**: reads files to extract N lines before/after matches (using a file cache). <br>• **Line truncation**: each match line truncated to 500 chars with `[truncated]` suffix. <br>• **Byte truncation**: total output capped at 50 KB. <br>• **Relative paths**: outputs paths relative to search directory. <br>• Abort signal support. |
| **Output** | `path:linenum: matchline` format with context lines as `path-linenum- contextline`. Actionable notices for limits. |

### 1.6 `find` — fd-powered file search

| Aspect | Detail |
|--------|--------|
| **Parameters** | `pattern` (string, required, glob), `path` (optional, default `.`), `limit` (optional, default 1000) |
| **Description** | Searches for files by glob pattern using fd. Respects .gitignore. |
| **Key behaviours** | • **Auto-downloads fd** via `ensureTool("fd")` if not available. <br>• Runs `fd --glob --color=never --hidden --max-results N`. <br>• **Gitignore respect**: explicitly passes `--ignore-file` for root and nested `.gitignore` files found via glob. <br>• **Result limit**: default 1000. <br>• **Byte truncation**: 50 KB cap. <br>• **Relative paths**: outputs paths relative to search directory. <br>• **Custom operations**: supports injecting a custom `glob()` implementation for remote/sandbox use. |
| **Output** | Newline-separated relative file paths. Notices for limits. |

### 1.7 `ls` — Directory listing

| Aspect | Detail |
|--------|--------|
| **Parameters** | `path` (optional, default `.`), `limit` (optional, default 500) |
| **Description** | Lists directory contents sorted alphabetically, with `/` suffix for directories. Includes dotfiles. |
| **Key behaviours** | • Case-insensitive alphabetical sort. <br>• Appends `/` to directory entries. <br>• **Entry limit**: default 500. <br>• **Byte truncation**: 50 KB cap. <br>• Skips entries that can't be stat'd. <br>• Returns `"(empty directory)"` for empty dirs. |
| **Output** | Newline-separated entries with `/` suffix for dirs. Notices for limits. |

### Pi Shared Infrastructure

| Module | Purpose |
|--------|---------|
| **truncate.js** | `truncateHead` (keep first N lines/bytes — for read/grep/find/ls), `truncateTail` (keep last N lines/bytes — for bash). Defaults: 2000 lines, 50 KB. `truncateLine` for grep (500 chars). Handles multi-byte UTF-8 correctly. |
| **path-utils.js** | `resolveToCwd` (~ expansion, absolute path handling, @ prefix stripping), `resolveReadPath` (adds macOS filename fixup variants: NFD, curly quotes, narrow-no-break-space AM/PM). `expandPath` for Unicode space normalization. |
| **edit-diff.js** | `fuzzyFindText` (exact → fuzzy matching), `normalizeForFuzzyMatch` (trailing whitespace, smart quotes, Unicode dashes, special spaces), `generateDiffString` (unified diff with line numbers + context), `computeEditDiff` (preview diff without applying), `stripBom`, `detectLineEnding`, `normalizeToLF`, `restoreLineEndings`. |
| **tools-manager.js** | `ensureTool("rg")`/`ensureTool("fd")` — auto-downloads ripgrep/fd binaries if not on PATH. |

---

## 2. Quecto's Built-in Tools (10 tools)

Quecto registers 6 **core** tools automatically and 4 **domain-specific** tools conditionally.

### 2.1 `exec` — Shell execution

| Aspect | Detail |
|--------|--------|
| **Parameters** | `command` (string, required) |
| **Description** | Executes a shell command in the workspace directory. |
| **Key behaviours** | • Spawns `sh -c <command>` via `tokio::process::Command` in workspace dir. <br>• **Timeout**: default 30 seconds (configurable), kills child on timeout. <br>• **Output capture limit**: default 1 MB, truncation annotation per-stream (stdout/stderr separate). <br>• **Env sanitization**: strips all `QUECTO_*` env vars (secrets), only passes allowlisted keys (`HOME`, `PATH`, `LANG`, `TZ`, `TERM`, `SHELL`, `USER`, `LOGNAME`, `TMPDIR`, `LC_*`). <br>• **Sandbox validation**: command validated against dangerous-pattern blocklist and optional allowlist before execution. <br>• **nsjail isolation mode**: optional Linux namespace jail with configurable memory/PID/CPU limits, network passthrough toggle, trusted binary path validation. Falls back to native if `allow_native_fallback=true`. <br>• **Separate stdout/stderr**: captured independently via async tasks, reported separately on error. <br>• **No stdin**: piped stdout/stderr, stdin not connected. <br>• **No streaming callback**: output is collected, not streamed to UI during execution. <br>• **No temp file**: truncated output is returned inline, no temp file path. |
| **Output** | stdout on success; `"exit code N\nstdout: ...\nstderr: ..."` on failure. |

### 2.2 `read_file` — Read file contents

| Aspect | Detail |
|--------|--------|
| **Parameters** | `path` (string, required, relative to workspace) |
| **Description** | Reads a file's text content. |
| **Key behaviours** | • Resolves relative to workspace via `workspace.join(path)`. <br>• **Sandbox validation**: path must resolve within workspace (follows symlinks). <br>• **Size limit**: rejects files > 1 MB. <br>• Reads as UTF-8 string (`tokio::fs::read_to_string`). <br>• **No offset/limit pagination**. <br>• **No image support**. <br>• **No macOS filename fixups**. <br>• **No truncation** (rejects oversized instead of truncating). |
| **Output** | Raw file content string. |

### 2.3 `write_file` — Write file contents

| Aspect | Detail |
|--------|--------|
| **Parameters** | `path` (string, required), `content` (string, required) |
| **Description** | Creates or overwrites a file. |
| **Key behaviours** | • Auto-creates parent directories (`create_dir_all`). <br>• Sandbox path validation. <br>• Returns byte count: `"wrote N bytes to path"`. |
| **Output** | Success message with byte count. |

### 2.4 `edit_file` — Find-and-replace edit

| Aspect | Detail |
|--------|--------|
| **Parameters** | `path` (string, required), `old` (string, required), `new` (string, required) |
| **Description** | Replaces first occurrence of a substring in a file. |
| **Key behaviours** | • Sandbox path validation + 1 MB size limit. <br>• Reads file, checks `content.contains(old)`, does `content.replacen(old, new, 1)`. <br>• **No fuzzy matching** — exact string match only. <br>• **No BOM handling**. <br>• **No line-ending normalization**. <br>• **No uniqueness enforcement** — silently replaces first occurrence even if multiple exist. <br>• **No diff generation**. <br>• **No no-op detection**. |
| **Output** | `"replaced 'old' with 'new' in path"` or error if substring not found. |

### 2.5 `append_file` — Append to file

| Aspect | Detail |
|--------|--------|
| **Parameters** | `path` (string, required), `content` (string, required) |
| **Description** | Appends content to a file (creates if doesn't exist). |
| **Key behaviours** | • Opens with `create(true).append(true)`. <br>• Sandbox path validation. <br>• Flushes after write. |
| **Output** | `"appended N bytes to path"`. |

### 2.6 `list_dir` — Directory listing

| Aspect | Detail |
|--------|--------|
| **Parameters** | `path` (string, required, relative to workspace) |
| **Description** | Lists directory contents. |
| **Key behaviours** | • Sandbox path validation. <br>• Appends `/` to directory entries. <br>• Sorts alphabetically (case-sensitive `sort()`). <br>• **No entry limit**. <br>• **No byte truncation**. <br>• **No dotfile mention** (includes all entries from `read_dir`). |
| **Output** | Newline-separated entries. |

### 2.7 `recall` — Retrieve spilled tool outputs

| Aspect | Detail |
|--------|--------|
| **Parameters** | `id` (string, required — spill ID or `"list"`) |
| **Description** | Retrieves previously collapsed/spilled tool outputs by ID. Supports `recall("list")` for a full index. |
| **Key behaviours** | • Part of Quecto's **context pruning** system — when tool outputs are large, they're spilled to a store and replaced with collapse stubs in the context. <br>• **Loop detection**: tracks recall counts per ID, warns at ≥3 recalls (model stuck in recall-collapse loop). <br>• **Index listing**: `recall("list")` returns all spilled entries with ID, tool, preview, and token count. <br>• Memory-capped: max 256 tracked IDs. |
| **Output** | Full spilled content, or index listing, or "not found" error. |

### 2.8 `message` — Send message to user's channel

| Aspect | Detail |
|--------|--------|
| **Parameters** | `text` (string, required), `target` (string, optional, e.g. `"telegram:12345"`) |
| **Description** | Sends a message to the user via their channel (Telegram etc.). |
| **Key behaviours** | • Uses `OutboundMessage` bus via `mpsc::Sender`. <br>• Falls back to `default_target` if no explicit target given. <br>• Errors if no target available. |

### 2.9 `spawn` — Spawn subagent process

| Aspect | Detail |
|--------|--------|
| **Parameters** | `task` (string, required), `agent_id` (string, optional), `system` (string, optional) |
| **Description** | Spawns a child `quecto agent` subprocess for background tasks. |
| **Key behaviours** | • **Agent ID validation**: alphanumeric + `_-`, 1-64 chars. <br>• **Allowlist enforcement**: optional list of permitted agent IDs. <br>• **Subprocess**: runs `quecto agent -m <task> -s <session_name>` with inherited base dir. <br>• **Timeout**: 120 seconds. <br>• **Stub mode**: if no base_dir configured, returns a stub result (no subprocess). <br>• **No output capture**: stdout/stderr are `/dev/null` — only exit code is returned. |

### 2.10 `cron` — Scheduled task management

| Aspect | Detail |
|--------|--------|
| **Parameters** | `action` (string, required: add/remove/list/enable/disable), plus action-specific fields |
| **Description** | CRUD for scheduled cron jobs with interval or cron-expression schedules. |
| **Key behaviours** | • **Add**: requires name, message, and either `interval_seconds` or `cron_expression`. Optional `deliver_to` target. Duplicate name check. <br>• **Remove/Enable/Disable**: by job name. <br>• **List**: shows all jobs with status, schedule, last run time, last error, delivery target. <br>• Backed by `FileCronStore`. |

### 2.11 `web_search` — Web search

| Aspect | Detail |
|--------|--------|
| **Parameters** | `query` (string, required) |
| **Description** | Searches the web using Brave Search API (with DuckDuckGo Instant Answer fallback). |
| **Key behaviours** | • **Brave** (if API key configured): queries `/res/v1/web/search`, extracts top 5 results with title/URL/description. <br>• **DuckDuckGo** (fallback): queries Instant Answer API, returns abstract or related topics. <br>• Custom base URLs for testing. |

### Quecto Shared Infrastructure

| Module | Purpose |
|--------|---------|
| **sandbox.rs** | Path validation (workspace restriction, symlink-following canonicalization), dangerous command blocklist (rm -rf /, fork bombs, dd, etc.), optional command allowlist, shell metacharacter detection. |
| **registry.rs** | `ToolRegistryImpl` — HashMap-based registry. `with_core_tools()` registers the 6 core tools. Dynamic registration via `register()`. |

---

## 3. Comparison Table

### 3.1 Core Dev Tools (Feature Parity Focus)

| Feature | Pi | Quecto | Gap |
|---------|:--:|:------:|:---:|
| **File Read** | ✅ `read` | ✅ `read_file` | **Partial** |
| ↳ Offset/limit pagination | ✅ 1-indexed, with continuation hints | ❌ | 🔴 Missing |
| ↳ Truncation (2000 lines / 50 KB) | ✅ head-truncation with notices | ❌ Rejects files > 1 MB, no truncation | 🔴 Missing |
| ↳ Image support (base64 + resize) | ✅ jpg/png/gif/webp, auto-resize | ❌ | 🔴 Missing |
| ↳ macOS filename fixups (NFD, curly quotes) | ✅ | ❌ | 🟡 Nice-to-have |
| ↳ Absolute & `~` path resolution | ✅ | ❌ Relative to workspace only | 🟡 By design (sandbox) |
| ↳ Abort/cancel support | ✅ AbortSignal | ❌ No cancel mechanism | 🟡 |
| **File Write** | ✅ `write` | ✅ `write_file` | **Parity ✅** |
| ↳ Auto-create parent dirs | ✅ | ✅ | ✅ |
| ↳ Reports bytes written | ✅ | ✅ | ✅ |
| ↳ Sandbox path validation | ❌ (no sandbox) | ✅ | Quecto ahead |
| **File Edit** | ✅ `edit` | ✅ `edit_file` | **Major gaps** |
| ↳ Exact text match | ✅ | ✅ | ✅ |
| ↳ Fuzzy matching (trailing WS, smart quotes, Unicode dashes/spaces) | ✅ | ❌ | 🔴 Critical |
| ↳ BOM handling (strip before match, restore after) | ✅ | ❌ | 🔴 |
| ↳ Line-ending normalization (CRLF ↔ LF) | ✅ Detect, normalize, restore | ❌ | 🔴 |
| ↳ Uniqueness enforcement (reject if >1 match) | ✅ Errors with count | ❌ Silently replaces first | 🔴 Critical |
| ↳ No-op detection (reject identical replacement) | ✅ | ❌ | 🟡 |
| ↳ Diff generation (unified diff with line numbers) | ✅ 4-line context | ❌ | 🟡 |
| ↳ Sandbox path validation | ❌ | ✅ | Quecto ahead |
| **Shell Execution** | ✅ `bash` | ✅ `exec` | **Partial** |
| ↳ Timeout | ✅ Optional, no default | ✅ Default 30s | ✅ |
| ↳ Output truncation | ✅ Tail-truncation (last 2000 lines / 50 KB) | ✅ 1 MB capture limit | ✅ Different strategy |
| ↳ Temp file for full output | ✅ `/tmp/pi-bash-*.log` | ❌ | 🟡 |
| ↳ Live streaming to TUI | ✅ `onUpdate` callback | ❌ | 🟡 |
| ↳ Process tree kill | ✅ `killProcessTree(pid)` | ✅ `child.kill()` | 🟡 Pi kills tree; Quecto kills child only |
| ↳ Env sanitization | ❌ Passes full `getShellEnv()` | ✅ Strips `QUECTO_*`, allowlist only | Quecto ahead |
| ↳ Dangerous command blocklist | ❌ | ✅ | Quecto ahead |
| ↳ nsjail sandbox isolation | ❌ | ✅ | Quecto ahead |
| ↳ Command prefix hook | ✅ | ❌ | 🟡 |
| ↳ Spawn hook (transform command/cwd/env) | ✅ | ❌ | 🟡 |
| ↳ Abort/cancel signal | ✅ | ❌ (timeout-based only) | 🟡 |
| **Grep/Search** | ✅ `grep` (ripgrep) | ❌ | 🔴 Missing |
| ↳ Auto-download rg binary | ✅ | ❌ | |
| ↳ Regex + literal + case-insensitive + glob filter | ✅ | ❌ | |
| ↳ Context lines | ✅ | ❌ | |
| ↳ Match limit (default 100) | ✅ | ❌ | |
| ↳ Line truncation (500 chars) | ✅ | ❌ | |
| **Find (file search)** | ✅ `find` (fd) | ❌ | 🔴 Missing |
| ↳ Auto-download fd binary | ✅ | ❌ | |
| ↳ Glob pattern, .gitignore respect | ✅ | ❌ | |
| ↳ Result limit (default 1000) | ✅ | ❌ | |
| **Directory Listing** | ✅ `ls` | ✅ `list_dir` | **Partial** |
| ↳ Dir suffix `/` | ✅ | ✅ | ✅ |
| ↳ Alphabetical sort | ✅ Case-insensitive | ✅ Case-sensitive | 🟡 |
| ↳ Entry limit (default 500) | ✅ | ❌ No limit | 🟡 |
| ↳ Byte truncation (50 KB) | ✅ | ❌ | 🟡 |
| ↳ Default path `.` | ✅ Optional, defaults to `.` | ❌ Required parameter | 🟡 |
| **File Append** | ❌ | ✅ `append_file` | Quecto ahead |

### 3.2 Domain-Specific Tools (Quecto only)

| Tool | Pi | Quecto | Notes |
|------|:--:|:------:|-------|
| **recall** (spilled context retrieval) | ❌ | ✅ | Part of Quecto's context-pruning system. Pi handles this via extensions/compaction. |
| **message** (send to user channel) | ❌ | ✅ | Telegram/channel integration. Pi is terminal-only (extensible via extensions). |
| **spawn** (subagent process) | ❌ | ✅ | Pi philosophy: "No sub-agents" built-in; use extensions. |
| **cron** (scheduled tasks) | ❌ | ✅ | Quecto has a gateway/daemon mode; Pi is interactive-only. |
| **web_search** (Brave/DDG) | ❌ | ✅ | Pi leaves this to extensions or bash `curl`. |
| **WASM tool runtime** | ❌ | ✅ | Quecto can load tools as WASM components with capability gating. Pi uses TypeScript extensions. |

### 3.3 Extensibility & Architecture

| Aspect | Pi | Quecto |
|--------|:--:|:------:|
| **Extension model** | TypeScript extensions (full API: tools, commands, shortcuts, events, UI) | WASM Component Model (WIT interface, capability-gated) |
| **Tool registration** | Extensions can register/replace any tool at runtime | `ToolRegistryImpl.register(Arc<dyn Tool>)` |
| **Custom operations injection** | All tools accept `options.operations` for testing/remote backends | Tools take `Arc<Sandbox>` + `Arc<PathBuf>` workspace |
| **Abort/cancel** | AbortSignal on all tools | Not implemented |
| **Output format** | `{content: ContentBlock[], details: any}` — structured multi-block | `{content: String, is_error: bool}` — flat string |
| **Truncation system** | Unified `truncateHead`/`truncateTail` with line+byte dual limits | Per-tool: 1 MB capture (exec), 1 MB reject (read/edit), none (list_dir) |
| **Path resolution** | `resolveToCwd` (cwd-relative, ~, @, absolute, macOS fixups) | `workspace.join(relative)` + sandbox validation |
| **Security** | None built-in (philosophy: use extensions/containers) | Sandbox (path validation, command blocklist/allowlist, nsjail) |
| **Tool auto-download** | `ensureTool("rg")`, `ensureTool("fd")` | Not applicable (no grep/find tools) |

---

## 4. Priority Gap Summary

### 🔴 Critical (needed for dev-tool parity)

1. **`grep` tool** — Without it, the model must use `exec` + manual `grep`/`rg`, wasting tokens and losing structured output.
2. **`find` tool** — Same rationale as grep; file discovery is a core workflow.
3. **Edit: uniqueness enforcement** — Silently replacing the first of N matches causes subtle bugs. Must reject ambiguous edits.
4. **Edit: fuzzy matching** — LLMs frequently produce slightly wrong whitespace, smart quotes, or Unicode dashes. Without fuzzy matching, edit calls fail unnecessarily.
5. **Read: offset/limit pagination** — Without pagination, large files (>1 MB) simply can't be read. This blocks real-world coding workflows.
6. **Read: truncation** — Rejecting files >1 MB is too aggressive. Should truncate and provide continuation hints like Pi does.

### 🟡 Important (quality/polish)

7. **Edit: BOM handling** — Windows-origin files with BOM will cause match failures.
8. **Edit: line-ending normalization** — CRLF files will cause match failures if LLM sends LF in oldText.
9. **Edit: diff generation** — Useful for TUI/review but not blocking.
10. **Edit: no-op detection** — Prevents wasted turns.
11. **Read: image support** — Needed for screenshot-driven workflows.
12. **Bash/exec: live streaming** — Important for long-running commands in interactive mode.
13. **Bash/exec: temp file for full output** — Useful for debugging.
14. **List: entry limit + byte truncation** — Prevents huge directories from blowing up context.
15. **List: default path** — Convenience.
16. **Bash/exec: process tree kill** — `child.kill()` may leave orphan grandchildren.
17. **Bash/exec: abort/cancel signal** — User-initiated cancellation.

### ✅ Quecto Advantages (keep these)

- **Sandbox** (path validation, command blocklist, nsjail isolation)
- **Env sanitization** (allowlist-based, strips secrets)
- **append_file** tool (Pi doesn't have this)
- **recall** tool (context spill/retrieval)
- **message/spawn/cron/web_search** (domain tools)
- **WASM tool runtime** (capability-gated isolation)
