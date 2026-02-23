# Context Pruning Design

## Problem

Quecto has no context window management. The agent loop sends the entire `Vec<Message>` plus every `ToolDefinition` to the LLM on every iteration. Tool results dominate context — in a real session analysis, 91.6% of tokens were tool output and only 6.3% were actual human/assistant conversation.

### Evidence: Real Session Analysis

We analysed a live session from the opencode SQLite database (`~/.local/share/opencode/opencode.db`). The session contained 3 user messages, 103 parts, and ~85,000 estimated tokens.

| Category | Parts | Bytes | ~Tokens | % of total |
|----------|-------|-------|---------|------------|
| webfetch (tool results) | 16 | 168,301 | ~42,000 | 49.4% |
| bash (tool results) | 16 | 110,100 | ~27,500 | 32.3% |
| read (tool results) | 1 | 27,308 | ~6,800 | 8.0% |
| task (tool results) | 1 | 9,004 | ~2,250 | 2.6% |
| text (human + assistant prose) | 21 | 21,384 | ~5,350 | 6.3% |
| step-start/finish (metadata) | 49 | 6,781 | ~1,700 | 2.0% |
| **Total** | **103** | **340,347** | **~85,000** | |

The single largest part was a `bash` call (`find -type d`) at 76KB (~19,156 tokens). The model extracted one file path from it and moved on. That output has zero value after the model's next response, yet it persists in context for the rest of the session.

## Design

### Core Rule: 3-Turn Collapse

After a tool result has been in context for 3 LLM round-trips, replace it with a one-line stub containing a `recall()` address. The full output is persisted to a spill file on disk.

**Why 3 turns:**

- Turn 0: Model calls the tool, receives the full result.
- Turn 1: Model responds — its response is the implicit summary/extraction of what mattered.
- Turn 2: Safety margin for immediate follow-up ("wait, go back to that output and also check...").
- Turn 3: Collapsed. The model's own prose from turn 1 carries the durable knowledge forward.

**What gets collapsed:**

- Tool result messages (role: `tool`) older than 3 turns.

**What never gets collapsed:**

- System messages.
- User messages.
- Assistant messages (the model's own prose, reasoning, and synthesis).

### Collapse Format

A tool result is replaced with a single line containing the tool name, a human-readable preview, the original token count, and a direct-address `recall()` identifier:

```
[bash: find ~/.local/share/opencode -type d (19,156 tokens) — recall("turn20:bash:0")]
```

The format is:

```
[{tool_name}: {first_60_chars_of_input} ({token_count} tokens) — recall("{turn_id}:{tool}:{index}")]
```

The `recall()` address is a deterministic key — not a search query. The model sees exactly what was there, how big it was, and the exact identifier to pass to `recall()` to retrieve it.

### Spill File

Full tool outputs are written to a JSONL spill file when they are first returned (not when collapsed — the write happens immediately so no data is ever lost).

**Location:** `<workspace>/sessions/<session_key>/spill.jsonl`

**Format:** One JSON object per line, append-only:

```jsonl
{"id":"turn2:webfetch:0","tool":"webfetch","input_preview":"https://blog.cloudflare.com/code-mode-mcp/","tokens":5289,"content":"...full output..."}
{"id":"turn2:webfetch:1","tool":"webfetch","input_preview":"https://platform.claude.com/docs/en/agents-and-tools/tool-use/tool-search-tool","tokens":6160,"content":"...full output..."}
{"id":"turn20:bash:0","tool":"bash","input_preview":"find ~/.local/share/opencode -type d","tokens":19156,"content":"...full output..."}
```

Fields:

| Field | Description |
|-------|-------------|
| `id` | Deterministic key: `turn{N}:{tool_name}:{index}` where index is the tool call's position within that turn (0-based) |
| `tool` | Tool name |
| `input_preview` | First 100 characters of the tool input (command, URL, file path, etc.) |
| `tokens` | Estimated token count of the full output |
| `content` | The complete, unmodified tool output |

The spill file is append-only. No updates, no deletes, no compaction. For a typical session this file will be a few MB at most.

### The `recall` Tool

A new built-in tool that retrieves spilled tool outputs by ID or lists all available entries.

**Definition:**

```
recall(id: string) -> string
```

**Behaviour:**

- If `id` is a spill key (e.g. `"turn20:bash:0"`): Open the session's spill file, find the matching entry, return its `content` field.
- If `id` is `"list"`: Return the full index of all spill entries — ID, tool name, input preview, and token count for each. No content. This lets the model browse the complete history when the manifest's last-10 isn't enough.
- If not found, return an error: `"No spilled output found for id: {id}"`.

**Important:** The `recall()` result is itself a tool result. It is subject to the same 3-turn collapse rule. If the model recalls something and doesn't use it within 3 turns, it collapses again. This prevents the model from re-inflating context by recalling everything.

### Spill Manifest (Pinned)

When stubs collapse or get dropped by the sliding window, the model loses its breadcrumbs. A pinned manifest message solves this — one system-level message that is always present in context and updated before each LLM call.

**Format:**

```
[Session memory: 47 spilled entries via recall()]
Oldest: turn2:webfetch:0 — cloudflare blog (5,289 tokens)
Latest: turn30:bash:3 — sqlite3 SELECT parts (673 tokens)
Recent:
  turn28:bash:0 — sqlite3 biggest parts (758 tokens)
  turn29:bash:0 — sqlite3 per-tool breakdown (692 tokens)
  turn30:bash:0 — sqlite3 text parts (673 tokens)
  turn30:bash:1 — sqlite3 text previews (306 tokens)
  turn30:bash:2 — find opencode dirs (19,156 tokens)
  ...
Use recall("<id>") to retrieve full content.
Use recall("list") for complete index.
```

**Properties:**

- **Pinned.** Never dropped by the sliding window, never collapsed.
- **Fixed budget.** Shows the last ~10 stubs plus summary metadata. Stays under ~500 tokens regardless of how long the session runs.
- **Updated in-place** before each LLM call. The agent loop rebuilds it from the spill store's index. No separate tracking — the spill file is the source of truth.
- **Not present when empty.** If no spill entries exist, the manifest message is not injected.

The manifest means the model always knows what's available, even after aggressive pruning. Individual stubs in the conversation are a nice-to-have (the model sees them in context near where they were used), but even if every stub gets dropped, the manifest provides the path back via `recall("list")` or a direct ID.

### Token Estimation

Use a byte-based heuristic: **1 token ~ 3 bytes**. This is intentionally conservative — we'd rather overestimate tokens and prune slightly early than underestimate and blow past the model's actual context limit.

The 1:4 ratio is accurate for English prose, but code, shell output, paths, and URLs have many single-character tokens (`{`, `;`, `\n`, `/`) that inflate the real token count relative to byte length. Since tool output (the primary pruning target) is predominantly code and paths, 1:3 is the safer default.

```rust
fn estimate_tokens(text: &str) -> usize {
    // Conservative: 1 token ≈ 3 bytes.
    // Overestimates for prose, roughly accurate for code/paths/URLs.
    // This is deliberate — better to prune early than to exceed the
    // model's context limit and get a provider error.
    text.len() / 3
}
```

**Headroom guidance:** Set `max_context_tokens` to ~80% of the model's actual context window. For a 128K model, use 100K. For a 200K model, use 160K. This accounts for estimation inaccuracy, tool definitions (which consume tokens but aren't in the message array), and provider overhead. The default of 100K assumes a 128K-class model.

### Turn Tracking

The agent loop already tracks iterations via its loop counter. Each iteration (one LLM call + tool execution round) is a "turn." The turn counter is used to:

1. Assign spill IDs: `turn{N}:{tool}:{index}`.
2. Determine which tool results are 3+ turns old.

The simplest implementation: stamp each tool result message with the turn number when it's appended to the message vec. On each new iteration, scan for tool result messages where `current_turn - stamped_turn >= 3` and replace their content with the one-liner stub.

## Worked Example

This is the actual session from the analysis above, showing context state at each turn with the 3-turn collapse rule applied.

### Turn 1 — User message
```
User: "read the ironclaw github repo... propose an approach
       to context management and tool security"               136 tokens
```
**Running context: ~136 tokens**

### Turn 2 — Parallel webfetch (3 URLs)
```
Assistant: "Let me fetch all three in parallel"                 46 tokens
  webfetch: ironclaw 404 (error)                                72 tokens
  webfetch: cloudflare blog                                  5,289 tokens  <- BIG
  webfetch: anthropic tool-search                            6,160 tokens  <- BIG
```
**Running context: ~11,703 tokens**

### Turn 3 — Retry ironclaw
```
Assistant: "ironclaw 404, trying variations"                    37 tokens
  webfetch: ironclaw 404 again                                  72 tokens
  webfetch: github search results                            2,356 tokens
```
**Running context: ~14,168 tokens**

### Turn 4 — More searches
```
  webfetch: github search #2                                 2,356 tokens
  webfetch: nicholasgasior 404                                  70 tokens
```
Turn 2 tool results collapse (3 turns ago):
```diff
- webfetch: cloudflare blog                                  5,289 tokens
+ [webfetch: https://blog.cloudflare.com/code-mode-mcp/ (5,289 tokens) — recall("turn2:webfetch:1")]     25 tokens

- webfetch: anthropic tool-search                            6,160 tokens
+ [webfetch: https://platform.claude.com/docs/... (6,160 tokens) — recall("turn2:webfetch:2")]            25 tokens
```
**Savings: 11,399 tokens. Running context: ~5,195 tokens**

### Turn 5 — Found nearai/ironclaw
```
  question: ask user about repo URL                            254 tokens
  webfetch: nearai/ironclaw README                           8,122 tokens  <- BIG
```
Turn 3 collapses: **savings 2,356 tokens. Running context: ~11,215 tokens**

### Turns 6-10 — More webfetch + task agent
Each turn fetches more data, previous fetches collapse on schedule.
Peak during this phase: ~14,000 tokens.

### Turn 11 — Big proposal text
```
Assistant: [writes the 3,331-token proposal]                 3,331 tokens
```
All tool results from turns 6-8 have collapsed. This text is an assistant message — it persists forever.
**Running context: ~8,500 tokens**

### Turn 12 — User pivots the conversation
```
User: "Are we overcomplicating compaction?"                      78 tokens
```
**Running context: ~8,578 tokens**

### Turn 13 — Simplified approach
```
Assistant: "You're right" [writes simpler design]            1,104 tokens
```
**Running context: ~9,682 tokens**

### Turn 14 — User asks to investigate opencode
```
User: "check the JSON logs of this session in opencode"         70 tokens
```
**Running context: ~9,752 tokens**

### Turns 15-30 — Database investigation (16 bash calls)
This is where the original session exploded to 85K tokens.

**Turn 20** is the critical moment — the `find -type d` command returns 19,156 tokens.

In the original session, this sits in context for the remaining ~15 turns.

With 3-turn collapse:
```
Turn 20: bash: find -type d                                 19,156 tokens  (model sees full output)
Turn 21: model acts on it                                               (still in context)
Turn 22: model may reference it                                         (still in context)
Turn 23: COLLAPSED
  [bash: find ~/.local/share/opencode -type d (19,156 tokens) — recall("turn20:bash:0")]    25 tokens
```

Similar pattern for the `read` of the session_diff JSON (6,827 tokens), the sqlite3 queries, etc.

**Peak context during investigation phase: ~28,000 tokens** (at turn 20, before the monster bash collapses).

**Steady-state during investigation: ~12,000-18,000 tokens.**

### Turns 31-37 — Pure text conversation
No tool calls. Just user messages and assistant responses. Nothing to collapse.
**Running context: ~14,000 tokens** (conversation prose only).

### Summary

| Metric | Original | With 3-turn collapse |
|--------|----------|---------------------|
| Peak context | ~85,000 tokens | ~28,000 tokens |
| Steady-state | Grows forever | ~12,000-18,000 tokens |
| Final context | ~85,000 tokens | ~14,000 tokens |
| Tokens saved | — | ~63,000 (~74%) |
| Biggest single win | — | `find -type d`: 19,156 -> 25 tokens |
| Conversation text | ~5,350 tokens (untouched) | ~5,350 tokens (untouched) |

## Implementation

### Files to Create/Modify

| Change | Layer | File | Est. Lines |
|--------|-------|------|------------|
| `ContextSpillStore` trait | domain | `src/domain/session.rs` | ~20 |
| `SpillEntry` struct | domain | `src/domain/session.rs` | ~10 |
| `estimate_tokens()` | application | `src/application/context_pruning.rs` (new) | ~10 |
| `collapse_old_tool_results()` | application | `src/application/context_pruning.rs` (new) | ~50 |
| `enforce_context_ceiling()` | application | `src/application/context_pruning.rs` (new) | ~40 |
| `build_spill_manifest()` | application | `src/application/context_pruning.rs` (new) | ~40 |
| Call pruning + manifest before each `chat()` | application | `src/application/agent_loop.rs` | ~15 |
| Spill on tool result append | application | `src/application/agent_loop.rs` | ~10 |
| `FileContextSpillStore` (JSONL) | infrastructure | `src/infrastructure/persistence/context_spill.rs` (new) | ~100 |
| `RecallTool` (with `"list"` support) | infrastructure | `src/infrastructure/tools/recall.rs` (new) | ~80 |
| Register `RecallTool` | infrastructure | `src/infrastructure/tools/registry.rs` | ~3 |
| Config fields | infrastructure | `src/infrastructure/config.rs` | ~5 |
| Turn stamp + pinned flag on `Message` | domain | `src/domain/message.rs` | ~8 |
| **Total** | | | **~391** |

### Domain Changes

Add to `Message`:

```rust
/// The agent loop turn number when this message was created.
/// Used by context pruning to determine when to collapse tool results.
/// None for messages loaded from session history (pre-existing).
pub turn: Option<u32>,

/// Whether this message is pinned (exempt from sliding window eviction).
/// True for: system messages, first user message, spill manifest.
pub is_pinned: bool,

/// Whether this message is the spill manifest (updated in-place each turn).
pub is_manifest: bool,

/// Whether this tool result has already been collapsed to a stub.
/// Prevents re-collapsing and avoids brittle content-sniffing heuristics.
pub is_collapsed: bool,
```

Add to `domain/session.rs`:

```rust
/// A single spilled tool output entry.
pub struct SpillEntry {
    pub id: String,           // "turn20:bash:0"
    pub tool: String,         // "bash"
    pub input_preview: String, // first 100 chars of tool input
    pub tokens: usize,
    pub content: String,      // full tool output
}

/// Index-only view of a spill entry (no content). Used for manifests and listing.
pub struct SpillIndex {
    pub id: String,
    pub tool: String,
    pub input_preview: String,
    pub tokens: usize,
}

pub trait ContextSpillStore: Send + Sync {
    fn append(
        &self,
        session_key: &str,
        entry: &SpillEntry,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>>;

    fn recall(
        &self,
        session_key: &str,
        id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<SpillEntry>, DomainError>> + Send + '_>>;

    /// Return index-only entries for all spilled outputs in this session.
    /// Used by the manifest builder and recall("list").
    fn list_entries(
        &self,
        session_key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SpillIndex>, DomainError>> + Send + '_>>;
}
```

### Application Changes

New file `src/application/context_pruning.rs`:

```rust
/// Estimate token count from byte length. Intentionally imprecise.
pub fn estimate_tokens(text: &str) -> usize {
    text.len() / 4
}

/// Format the one-liner stub for a collapsed tool result.
pub fn collapse_stub(tool: &str, input_preview: &str, tokens: usize, spill_id: &str) -> String {
    let preview = truncate_utf8_safe(input_preview, 60);
    format!("[{tool}: {preview} ({tokens} tokens) — recall(\"{spill_id}\")]")
}

/// Truncate a string to at most `max_chars` characters, appending "..."
/// if truncated. Safe for multi-byte UTF-8 — never splits a character.
fn truncate_utf8_safe(s: &str, max_chars: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars - 3).collect();
        format!("{truncated}...")
    }
}

/// Scan messages and collapse tool results that are 3+ turns old.
/// Returns the number of tool results collapsed.
pub fn collapse_old_tool_results(
    messages: &mut [Message],
    current_turn: u32,
    collapse_after: u32,  // default: 3
) -> usize {
    let mut collapsed = 0;
    for msg in messages.iter_mut() {
        if msg.role != Role::Tool || msg.is_collapsed {
            continue;
        }
        if let Some(turn) = msg.turn {
            if current_turn.saturating_sub(turn) >= collapse_after {
                msg.content = collapse_stub(
                    &msg.tool_name.as_deref().unwrap_or("tool"),
                    &msg.input_preview.as_deref().unwrap_or(""),
                    estimate_tokens(&msg.content),
                    &msg.spill_id.as_deref().unwrap_or("unknown"),
                );
                msg.is_collapsed = true;
                collapsed += 1;
            }
        }
    }
    collapsed
}

/// Build or update the pinned spill manifest message.
/// Shows the last 10 spill entries plus summary metadata.
/// Fixed token budget (~500 tokens) regardless of session length.
pub async fn update_spill_manifest(
    messages: &mut Vec<Message>,
    spill_store: &dyn ContextSpillStore,
    session_key: &str,
) {
    let entries = spill_store.list_entries(session_key).await.unwrap_or_default();
    if entries.is_empty() {
        // Remove manifest if it exists and there are no entries
        messages.retain(|m| !m.is_manifest);
        return;
    }

    let total = entries.len();
    let oldest = &entries[0];
    let latest = &entries[total - 1];
    let recent: Vec<_> = entries.iter().rev().take(10).collect();

    let mut manifest = format!(
        "[Session memory: {} spilled entries via recall()]\n\
         Oldest: {} — {} ({} tokens)\n\
         Latest: {} — {} ({} tokens)\n\
         Recent:\n",
        total,
        oldest.id, oldest.input_preview, oldest.tokens,
        latest.id, latest.input_preview, latest.tokens,
    );
    for entry in recent.iter().rev() {
        manifest.push_str(&format!(
            "  {} — {} ({} tokens)\n",
            entry.id, entry.input_preview, entry.tokens
        ));
    }
    manifest.push_str("Use recall(\"<id>\") to retrieve. Use recall(\"list\") for full index.");

    // Find existing manifest message and update, or insert one
    if let Some(msg) = messages.iter_mut().find(|m| m.is_manifest) {
        msg.content = manifest;
    } else {
        let mut msg = Message::system(manifest);
        msg.is_pinned = true;
        msg.is_manifest = true;
        // Insert after the system prompt but before conversation
        let pos = messages.iter().position(|m| m.role != Role::System).unwrap_or(0);
        messages.insert(pos, msg);
    }
}
```

In `src/application/agent_loop.rs`, the loop changes from:

```
loop {
    let response = provider.chat(request).await?;
    // ... handle tool calls ...
    // ... append tool results ...
}
```

To:

```
loop {
    // Collapse old tool results before each LLM call
    let collapsed = collapse_old_tool_results(&mut messages, current_turn, 3);
    if collapsed > 0 {
        tracing::info!(target: "context_prune", collapsed, turn = current_turn, "collapsed tool results");
    }

    // 2. Enforce hard context ceiling
    let dropped = enforce_context_ceiling(&mut messages, max_context_tokens, ...);

    // 3. Update the pinned spill manifest
    update_spill_manifest(&mut messages, &spill_store, &session_key).await;

    let response = provider.chat(request).await?;
    // ... handle tool calls ...

    // For each tool result:
    //   1. Assign turn number and spill ID
    //   2. Write full output to spill store
    //   3. Append to messages as normal
    for (index, tool_result) in results.iter().enumerate() {
        let spill_id = format!("turn{}:{}:{}", current_turn, tool_name, index);
        let entry = SpillEntry {
            id: spill_id.clone(),
            tool: tool_name.to_string(),
            input_preview: extract_input_preview(&tool_call),
            tokens: estimate_tokens(&tool_result.content),
            content: tool_result.content.clone(),
        };
        spill_store.append(&session_key, &entry).await?;

        let mut msg = Message::tool(tool_call_id, tool_result.content.clone());
        msg.turn = Some(current_turn);
        msg.spill_id = Some(spill_id);
        msg.input_preview = Some(entry.input_preview);
        messages.push(msg);
    }

    current_turn += 1;
}
```

### Infrastructure: FileContextSpillStore

```rust
pub struct FileContextSpillStore {
    base_dir: PathBuf,
}

impl FileContextSpillStore {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    fn spill_path(&self, session_key: &str) -> PathBuf {
        self.base_dir
            .join("sessions")
            .join(sanitize_filename(session_key))
            .join("spill.jsonl")
    }
}
```

`append()`: Create parent dirs if needed, open file in append mode, serialize `SpillEntry` as one JSON line, write + newline.

`recall()`: Open file, scan line by line, deserialize each line, return the first where `id` matches. This is O(n) in lines but spill files are small (hundreds of lines max per session). If this ever becomes a bottleneck, add a companion `.idx` file mapping IDs to byte offsets.

### Infrastructure: RecallTool

```rust
pub struct RecallTool {
    spill_store: Arc<dyn ContextSpillStore>,
    session_key: String,
}
```

- **Name:** `recall`
- **Description:** `"Retrieve a previously collapsed tool output by its ID. Use the ID shown in collapse stubs like: recall(\"turn20:bash:0\"). Use recall(\"list\") for the full index."`
- **Parameters:** `{ "id": { "type": "string", "description": "The spill ID from the collapse stub, or \"list\" for the full index" } }`
- **Execute:** Call `spill_store.recall(session_key, id)`, return `content` field or error.

**Repeated recall diagnostic:** The `RecallTool` tracks recall counts per ID within the session (a simple `HashMap<String, u32>` on the struct). If the same ID is recalled 3+ times, emit a `tracing::warn!` with `target: "context_prune"`:

```
warn!(target: "context_prune", id = "turn5:bash:0", recall_count = 3, 
      "repeated recall — model may be stuck in a recall-collapse loop");
```

This is diagnostic only — never block the recall. But it signals that the model is failing to extract what it needs from the output and may be thrashing. Useful for tuning the `collapse_after_turns` value or identifying tool outputs that need a different handling strategy (e.g., pre-structured output from the tool itself).

### Config

Add to `Config`:

```rust
/// Number of turns after which tool results are collapsed. Default: 3.
pub context_collapse_after_turns: u32,

/// Maximum context window size in estimated tokens.
/// When exceeded, oldest non-pinned messages are dropped.
/// Default: 100_000. Set lower for cheaper models with smaller windows,
/// higher for models with large context (e.g. 200K for Claude).
/// Also configurable via `--max-context-tokens` CLI flag and
/// `QUECTO_MAX_CONTEXT_TOKENS` env var.
pub max_context_tokens: usize,
```

`max_context_tokens` resolution order (highest priority first):
1. `--max-context-tokens` CLI flag (on `quecto agent` and REPL)
2. `QUECTO_MAX_CONTEXT_TOKENS` env var
3. `max_context_tokens` in `config.json`
4. Default: `100_000`

The spill file location derives from `base_dir` + session key — no config needed.

### New Dependencies

None. Uses `serde_json` (already a dependency) and `tokio::fs` (already a dependency).

## BDD Scenarios

### Feature: Context Pruning

```gherkin
@wip
Feature: Context pruning via 3-turn collapse

  Background:
    Given a configured agent with context pruning enabled

  Scenario: Tool results are preserved for 3 turns
    When the agent executes a bash tool on turn 1
    And the agent completes turn 2
    And the agent completes turn 3
    Then the tool result from turn 1 is still in full context

  Scenario: Tool results collapse after 3 turns
    When the agent executes a bash tool on turn 1
    And the agent completes turn 2
    And the agent completes turn 3
    And the agent completes turn 4
    Then the tool result from turn 1 is replaced with a collapse stub
    And the collapse stub contains the tool name "bash"
    And the collapse stub contains the estimated token count
    And the collapse stub contains the recall ID "turn1:bash:0"

  Scenario: Full tool output is spilled to disk on creation
    When the agent executes a bash tool on turn 1
    Then the spill file contains an entry with id "turn1:bash:0"
    And the spill entry content matches the original tool output

  Scenario: Recall retrieves spilled output
    Given a spilled tool result with id "turn5:bash:0"
    When the agent calls recall with id "turn5:bash:0"
    Then the recall result contains the full original output

  Scenario: Recall result is itself subject to collapse
    Given a spilled tool result with id "turn5:bash:0"
    When the agent calls recall with id "turn5:bash:0" on turn 10
    And the agent completes turns 11 through 13
    Then the recall result from turn 10 is replaced with a collapse stub

  Scenario: User and assistant messages are never collapsed
    When the agent processes 20 turns of mixed tool and text messages
    Then all user messages remain in full context
    And all assistant messages remain in full context

  Scenario: System messages are never collapsed
    Given a system prompt in the conversation
    When the agent processes 20 turns
    Then the system message remains in full context

  Scenario: Already-collapsed stubs are not re-collapsed
    When the agent executes a bash tool on turn 1
    And the agent completes turns 2 through 10
    Then the collapse stub from turn 1 appears exactly once
    And the message is_collapsed field is true
    And its content has not been modified since turn 4

  Scenario: Recall with unknown ID returns error
    When the agent calls recall with id "nonexistent:id:0"
    Then the recall result is an error containing "No spilled output found"

  Scenario: Recall list returns full index
    Given 5 spilled tool results
    When the agent calls recall with id "list"
    Then the result contains all 5 spill entry IDs
    And the result contains tool names and token counts
    And the result does not contain full content

  Scenario: Repeated recall emits diagnostic warning
    Given a spilled tool result with id "turn5:bash:0"
    When the agent calls recall with id "turn5:bash:0" three times
    Then a warning is logged with target "context_prune"
    And the warning contains "repeated recall"
    And the warning contains recall_count 3

  Scenario: Spill manifest is injected after first spill
    Given no spill entries exist
    When the agent executes a bash tool on turn 1
    Then a pinned manifest message appears in context
    And the manifest contains "1 spilled entries via recall()"

  Scenario: Spill manifest shows last 10 entries
    Given 25 spilled tool results
    Then the manifest lists the 10 most recent entries
    And the manifest shows total count as 25
    And the manifest shows the oldest and latest entry IDs

  Scenario: Spill manifest survives sliding window
    Given max_context_tokens is set to 500
    And 20 spilled tool results
    When the sliding window drops messages to fit budget
    Then the manifest message remains in context
    And the manifest is pinned

  Scenario: Spill manifest is updated in-place
    When the agent executes tools on turns 1 through 5
    Then only one manifest message exists in context
    And it reflects all 5 spill entries

  Scenario: No manifest when no spill entries exist
    When the agent processes 3 turns with no tool calls
    Then no manifest message exists in context
```

## What This Does Not Do

- **No LLM-based summarisation.** The model's own response after seeing tool output is the implicit summary. No extra LLM call, no latency, no cost.
- **No tool result truncation at insertion time.** The model always sees the full output for 3 turns. Pre-truncation can be added independently as a separate concern (for outputs that are dangerously large even for a single turn).
- **No changes to the provider trait.** `ChatRequest` still takes `&[Message]`. The pruning happens on the `Vec<Message>` before it's borrowed.
- **No new dependencies.** JSONL spill uses `serde_json` + `tokio::fs`, both already in the dependency tree.

## Sliding Window: Hard Context Ceiling

The 3-turn collapse handles the common case — tool results age out on schedule. But it doesn't cover the pathological case: the model does 30 tool calls in a row and even the "current 3 turns" window exceeds the provider's context limit.

The sliding window is the safety net. It enforces a hard ceiling on total context size.

### Rule

When total estimated tokens exceed `max_context_tokens` (configurable, default: 100,000), drop the oldest non-pinned messages until the total is back under budget. The ceiling is configurable via `config.json`, env var, or CLI flag — see Config section above.

**Pinned messages (never dropped):**

- System messages.
- The first user message in the conversation (the original task/request).

**Everything else slides:** user messages, assistant messages, tool results (whether full or already collapsed to stubs). Oldest first.

### Interaction with 3-Turn Collapse

The two mechanisms run in sequence before each LLM call:

```
1. collapse_old_tool_results()    — time-based, shrinks stubs
2. enforce_context_ceiling()      — size-based, drops messages
```

In practice, the 3-turn collapse does most of the work. The sliding window rarely fires because collapsed stubs are only ~25 tokens each. It exists for edge cases:

- A tool returns 200K tokens in a single result (exceeds the entire budget on turn 0).
- The model has a very long multi-turn conversation with dozens of exchanges that are all text (no tool results to collapse).
- A flurry of tool calls in turns N, N+1, N+2 that are all still "live" (within 3 turns) but collectively exceed the budget.

### Dropped Message Handling

Messages dropped by the sliding window follow the same spill pattern as collapsed tool results:

- **Tool result messages** that haven't been spilled yet get spilled before removal. Their stub remains in the conversation (or is removed entirely if even the stub doesn't fit).
- **User/assistant messages** are spilled with a synthetic ID: `turn{N}:text:0`. Their stub:

```
[user message (342 tokens) — recall("turn14:text:0")]
```

- If the conversation is so long that even stubs are over budget, stubs are dropped entirely. The spill file is the permanent record; the conversation becomes a sliding window over recent history with the manifest of available recalls.

### Implementation

Add to `src/application/context_pruning.rs`:

```rust
/// Enforce a hard ceiling on total context tokens.
/// Drops oldest non-pinned messages until under budget.
/// Returns the number of messages dropped.
///
/// Uses a two-pass approach to avoid O(n²) repeated scanning:
/// 1. Calculate total tokens and identify droppable message indices.
/// 2. Walk droppable indices from oldest, marking for removal until under budget.
/// 3. Single retain() pass to remove all marked messages.
pub fn enforce_context_ceiling(
    messages: &mut Vec<Message>,
    max_tokens: usize,
    spill_store: &dyn ContextSpillStore,
    session_key: &str,
) -> usize {
    let mut total = estimate_total_tokens(messages);
    if total <= max_tokens {
        return 0;
    }

    // Collect indices of droppable messages (oldest first, already in order)
    let droppable: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| !m.is_pinned)
        .map(|(i, _)| i)
        .collect();

    // Mark messages for removal until under budget
    let mut to_drop = HashSet::new();
    for &idx in &droppable {
        if total <= max_tokens {
            break;
        }
        total -= estimate_tokens(&messages[idx].content);
        // Spill if not already spilled (tool results with no spill_id,
        // or user/assistant messages being evicted)
        // ... (spill logic)
        to_drop.insert(idx);
    }

    let dropped = to_drop.len();
    let mut idx = 0;
    messages.retain(|_| {
        let keep = !to_drop.contains(&idx);
        idx += 1;
        keep
    });
    dropped
}

fn estimate_total_tokens(messages: &[Message]) -> usize {
    messages.iter().map(|m| estimate_tokens(&m.content)).sum()
}
```

### Agent Loop Integration

The agent loop becomes:

```
loop {
    // 1. Collapse tool results older than 3 turns
    let collapsed = collapse_old_tool_results(&mut messages, current_turn, 3);

    // 2. Enforce hard ceiling
    let dropped = enforce_context_ceiling(&mut messages, max_context_tokens, ...);

    if collapsed > 0 || dropped > 0 {
        tracing::info!(
            target: "context_prune",
            collapsed, dropped, turn = current_turn,
            total_tokens = estimate_total_tokens(&messages),
            "context pruned"
        );
    }

    let response = provider.chat(request).await?;
    // ... rest of loop ...
}
```

### BDD Scenarios

```gherkin
  Scenario: Sliding window drops oldest messages when over budget
    Given max_context_tokens is set to 1000
    When the agent accumulates 2000 tokens of messages
    Then the oldest non-pinned messages are dropped
    And total context is under 1000 tokens

  Scenario: System messages are never dropped
    Given max_context_tokens is set to 500
    And a system prompt consuming 200 tokens
    When the agent accumulates 800 tokens of messages
    Then the system message remains in context
    And non-system messages are dropped to fit

  Scenario: First user message is pinned
    Given max_context_tokens is set to 500
    When the agent accumulates 800 tokens across 5 user messages
    Then the first user message remains in context
    And later user messages may be dropped

  Scenario: Dropped messages are spilled
    Given max_context_tokens is set to 1000
    When the sliding window drops a user message
    Then the spill file contains the dropped message content
    And the dropped message has a valid recall ID
```

## Future Extensions

These are not part of this design but are natural follow-ons:

1. **Tool result truncation at insertion.** For outputs exceeding a configurable ceiling (e.g. 16KB / ~4,000 tokens), truncate to head + tail with a middle marker even on the first turn. Full output still goes to spill. This prevents a single tool result from consuming the entire context window.

2. **Deferred tool loading.** When tool count exceeds ~10, only send core tool definitions to the LLM and let it discover extended tools via a `search_tools` meta-tool. Reduces the fixed token cost of tool definitions.

3. **Spill file indexing.** If sessions grow to thousands of spilled entries, add a `.idx` companion file mapping IDs to byte offsets for O(1) recall lookup instead of O(n) line scan.
