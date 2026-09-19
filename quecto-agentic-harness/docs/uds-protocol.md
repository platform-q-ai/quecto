# UDS Protocol Reference

Quecto's UDS (Unix Domain Socket) mode runs a persistent agent process that accepts length-prefixed JSON commands over a local socket. This is the integration point for TUIs, IDE plugins, web UIs, Telegram bots, and any external automation.

## Starting the agent

```bash
quecto agent --mode uds
# stderr: quecto-agent-socket: /tmp/quecto-agent-<uuid>.sock

# Keep the agent alive even when all clients disconnect
quecto agent --mode uds --persist
```

The agent prints the socket path to stderr on startup. Options:

| Flag | Description |
|---|---|
| `--mode uds` | Required. Run in UDS event-bus mode instead of one-shot |
| `--socket <path>` | Explicit socket path (max 104 bytes). Default: auto-generated in `$XDG_RUNTIME_DIR` or `$TMPDIR` |
| `--session <name>` | Named session for persistence across restarts |
| `--no-session` | Ephemeral mode — no session saved to disk |
| `--system <text>` | Inject a system prompt (not persisted in session history) |
| `--persist` | Stay alive when all clients disconnect. Default: agent exits when the last client disconnects. Top-level only: a launcher-created child (`--parent-control`) is lifetime-bound to its launcher and refuses `--persist` (#1937) |

## Wire format

- **Transport:** Unix domain socket (stream)
- **Framing:** Length-prefixed UTF-8 JSON frames (ADR-0008), with dual-mode readers that also accept legacy `\n`-delimited JSON lines during the deprecation window. Shared bound: **8 MiB** per message (`quecto-line-io::PROTOCOL_LINE_CAP_BYTES`, including the trailing newline on legacy lines)
- **Direction:** Client sends **commands**, agent emits **events**
- **Multi-client:** Multiple clients can connect simultaneously. Events are broadcast to all clients; commands from all clients merge into a single serial dispatch loop
- **Shutdown:** By default the agent exits when all clients disconnect. Pass `--persist` to keep it running. A launcher-created child ignores client churn and ends on the loss of its launch-bound parent connection instead (#1935/#1937). Socket file is removed on exit
- **Security:** Socket file is created with `chmod 0600` (owner-only). On startup, dead auto-generated sockets are reaped by liveness check; the 24h age threshold is a fallback for sockets whose liveness cannot be determined
- **See also:** [ADR-0008](architecture-design-records/adr-0008-length-prefixed-uds-framing-and-bounded-events.md) for version negotiation and the NDJSON deprecation window, and the [protocol capability matrix](architecture/protocol-capability-matrix.md) for the current compatibility/evolution map
- **Session commands (#1968):** `list_sessions`, `search_session_metadata` (#2010), `resume_session`, `new_session`, `persist_session`, `clear_history`, `rewind_to`, `get_messages`, `get_message`, `sync`, `get_report` and `get_session_stats` are each answered by one sessions use case (named in the command's section and in [sessions.md](sessions.md#architecture-epic-1968)); the epic that moved them changed no command, field or refusal text

## Correlation IDs

Every command accepts an optional `id` field (string). When present, the corresponding `response` event echoes it back. This lets clients match responses to requests when multiple commands are in flight.

```json
{"type":"get_state","id":"req-42"}
```
```json
{"type":"response","id":"req-42","command":"get_state","success":true,"data":{...}}
```

---

## Commands (client → agent)

### `prompt`

Send a user message to the agent. This is the primary command — it triggers an LLM call, possible tool executions, and streams results back as events.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"prompt"` | yes | |
| `id` | string | no | Correlation ID |
| `message` | string | yes | The user message |
| `streamingBehavior` | `"steer"` \| `"followUp"` | when agent is running | How to handle this prompt if the agent is already processing a previous one |

**Behavior:**
- When the agent is idle, starts a new run immediately
- When the agent is already running and `streamingBehavior` is:
  - `"steer"` — cancels the current run after the active tool, then processes this message
  - `"followUp"` — queues this message to run after the current run completes
  - omitted — returns an error: `"agent is running; provide streamingBehavior"`

**Events emitted** (in order for a successful run):

1. `agent_start` — run begins
2. `turn_start` — LLM call begins
3. `token` (zero or more) — incremental answer-text streaming tokens
4. `thinking` (zero or more) — display-safe model reasoning/thinking deltas, separate from answer text
5. `tool_execution_start` / `tool_execution_end` (if tools are called)
5. `turn_end` — LLM call completed, includes assistant message
6. `agent_end` — run finished; carries `messageRefs` for this run (legacy `messages` is empty after #1060)
7. `response` with `command: "prompt"` and `success: true`

On error, emits `response` with `command: "agent_error"` instead of steps 6-7.

After completion, any pending follow-up or steer messages are automatically processed (each triggering its own event sequence).

**Example:**

```json
{"type":"prompt","id":"p-1","message":"What files are in the current directory?"}
```

---

### `steer`

Interrupt the current agent run and deliver a new message. If the agent is idle, the message is queued for the next prompt.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"steer"` | yes | |
| `id` | string | no | Correlation ID |
| `message` | string | yes | The new instruction |

**Behavior:**
- **Agent running:** Fires the cancellation signal (interrupts after the current tool completes), then prepends this message to the pending queue so it runs next
- **Agent idle:** Queues the message. It will execute after the next `prompt` completes

**Response:** `success: true` acknowledges the steer, not proof that the model has acted on it or that the prior run was cancelled. Forwarded controls carrying `ack: "accept"` are acknowledged only after admission to the dispatch queue. A full or closed queue returns `success: false` with an explicit delivery error; `agent_cmd` propagates that as a tool error. Forwarded prompt, steer and follow-up commands retain the request ID through dispatch. The immediate admission response includes `data.status: "accepted"`; later dispatch responses retain that ID (a prompt may dispatch as `follow_up`). A pending-buffer response carries `data.status: "queued"`; a full pending buffer returns `success: false`. Buffered idle work and earlier queued follow-ups yield while steering awaits dispatch; unrelated follow-ups cannot clear its priority. Each admitted steer retains priority until its own handler begins. Malformed steering commands are rejected by the typed command parser before they can cancel a turn or create pending intent. Retrieve the coordinator’s report to verify clarification handling; a completed dispatch response is not proof of successful task execution.

**Example:**

```json
{"type":"steer","id":"s-1","message":"Actually, focus on Python files only"}
```

---

### `follow_up`

Queue a message that will be processed after the current (or next) agent run completes. Unlike `steer`, this does not interrupt the running agent.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"follow_up"` | yes | |
| `id` | string | no | Correlation ID |
| `message` | string | yes | The follow-up message |

**Behavior:**
- **Agent running:** Appends the message to the pending queue. When the current run completes (success, error, or cancellation), pending messages are drained and executed in order.
- **Agent idle:** Appends the message to the pending queue and immediately starts draining it, matching Pi-style follow-up delivery.
- Each pending message triggers its own full event sequence (`agent_start` → `agent_end`)

**Response:** Always `success: true`.

**Example:**

```json
{"type":"follow_up","id":"fu-1","message":"Now summarize what you found"}
```

---

### `abort`

Cancel the current agent run. **`abort` is a full stop**, not a pause: the agent
stops completely and does **not** resume on its own. If the agent is idle, this is
a no-op.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"abort"` | yes | |
| `id` | string | no | Correlation ID |

**Behavior:**
- **Agent running:** Fires the cancellation signal. The agent stops after the
  current tool completes, in-flight tool calls and their child processes (e.g. a
  long `bash`) are terminated via the process group. The cancelled run's user
  message remains in history so the next prompt sees the same interrupted turn
  the client displayed; any assistant/tool output appended after that user
  message is discarded. `abort` is not a privacy delete: clients should not use
  it to remove sensitive text that was already submitted to the agent/model.
- **Workflow auto-continue is suppressed:** if the agent is bound to a workflow,
  `abort` clears any queued work and prevents the workflow auto-continue nudge
  from re-driving it. The agent stays stopped until explicitly re-driven by a
  fresh `prompt`. There is no "abort but keep going" mode — a resumable pause
  would be a separate command, never an overload of `abort`.
- **Agent idle:** No-op (acknowledged successfully)

**Response:** Always `success: true`.

**Example:**

```json
{"type":"abort","id":"ab-1"}
```

---

### `set_workflow_automation`

Toggle core workflow automation for this UDS session. Requires workflow mode.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"set_workflow_automation"` | yes | |
| `id` | string | no | Correlation ID |
| `autoContinue` | boolean | no | Enable/disable core auto-continue nudges |
| `completionNudge` | boolean | no | Enable/disable core completion nudges |

**Response data:**

```json
{"autoContinue": true, "completionNudge": true}
```

---

### `clear_history`

Clear the conversation history in-place without restarting the agent. The system prompt is preserved; all user, assistant, and tool messages are removed. Any pending follow-up/steer messages are drained. The session's retained-context namespace is also cleared so that stale tool output summaries are not re-injected on the next prompt. Owner: `ClearConversation` (#1864, #1968).

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"clear_history"` | yes | |
| `id` | string | no | Correlation ID |

**Behavior:**
- **Agent idle:** Clears all messages except the system prompt. Drains the pending queue. Clears the retention namespace (index + disk file). Saves the cleared session. Emits `ledger_advanced` with a new `epoch`, then returns `success: true`
- **Agent running:** Returns `success: false` with error `"cannot clear history while agent is running"`
- **Save failure:** the conversation is cleared and `ledger_advanced` is still emitted; the response is `success: false` with `"failed to save cleared session: <store error>"`

The system prompt (injected via `--system` flag) is preserved at `messages[0]`. Context-pruning manifests (`is_manifest = true`) and spill indices are **not** preserved — `recall("list")` returns empty after clear.

**Response:**

```json
{"type":"response","id":"ch-1","command":"clear_history","success":true}
```

**Error (agent is streaming):**

```json
{"type":"response","id":"ch-1","command":"clear_history","success":false,"error":"cannot clear history while agent is running"}
```

**Example:**

```json
{"type":"clear_history","id":"ch-1"}
```

---

### `list_sessions`

Discover persisted sessions, newest first. Owner: `ListSessions` (#1861,
#1968, #2009). Listing is not authorization to restore a row.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"list_sessions"` | yes | |
| `id` | string | no | Correlation ID |
| `scope` | `"local"` or `"global"` | no | Defaults to local; unsupported values are rejected |

**Response data:**

```json
{
  "scope": "local",
  "sessions": [
    {"key":"chat-1765930000-1a2b3c","title":"Fix the flaky test","messageCount":12,"updatedUnixSecs":1765930000,"updatedAt":1765930000,"homeState":"scoped","executionPath":"/work/project","resumeEligible":true}
  ],
  "diagnostics": [],
  "diagnosticsTotal": 0,
  "rebuilt": false
}
```

| Row field | Type | Description |
|---|---|---|
| `key` | string | Stable opaque identity, passed unchanged to `resume_session` |
| `title` | string | First user message, bounded for display; `(untitled)` when absent |
| `messageCount` | integer | Persisted user/assistant messages |
| `updatedUnixSecs` | integer \| null | Last modification time in Unix seconds |
| `updatedAt` | integer \| null | Retained alias of `updatedUnixSecs` |
| `homeState` | string | `scoped`, `legacy_unscoped`, or `unavailable` |
| `executionPath` | string \| null | Safely rendered execution directory, not a shell command |
| `resumeEligible` | boolean | Advisory same-execution-directory admission; resume rechecks authority |
| `homeVersion` | string | Opaque version (`h1-…`) of **this session's** home metadata as the row was listed: a digest of the session identity and every home fact, so no two rows share one and a token names exactly one record. Echo it as `resume_session.expectedHomeVersion` (#2011); it is the token the resume transaction recomputes from the authority and compares |

Local scope groups the nearest Git repository and related worktrees using Git
facts and canonical paths. Outside Git it matches the canonical exact folder.
Grouped worktrees can have different execution directories and are not thereby
eligible for restore. Global lists all saved identities, including legacy
unassociated and unavailable-home records. Metadata search is its own command,
[`search_session_metadata`](#search_session_metadata). Malformed records and discovery/catalogue failures produce diagnostics;
one bad record does not hide valid siblings. A malformed record's diagnostic
names its file so it can be repaired, e.g.
`cli_slippery-keith.json: session record unavailable: expected value at line 79 column 6`.
Diagnostics are **bounded per answer** (here and in `search_session_metadata`):
`diagnostics` carries the first 20 lines and then one `… and N more` line,
`diagnosticsTotal` how many there were in all — a store with a thousand corrupt
records does not put a thousand lines into every answer. A record that could
not be READ (an I/O failure, not a verdict on its content) is named the same
way for that answer only and read again on the next query. `rebuilt` reports derived catalogue
recovery — an unreadable or version-incompatible index, with a diagnostic — not
transcript modification: an absent index (first use) is built silently and an
index superseded by newer authority (a routine autosave) is refreshed silently.
The index (`home.catalogue`, version 2) records per-file stamps, home
observations and listing summaries, never transcript content or digests: a
process seeds from it and reads only records whose stamp changed, so the first
`list_sessions` of a new harness process over a large, unchanged directory
costs one `stat` per file (an index of the previous version is rebuilt once,
reported as `rebuilt`). Exact-key resume does not rely on the catalogue.

The TUI `/resume` picker defaults to Local, with a visible Local/Global control.
Tab/Shift+Tab move between scope, query and results; Enter/Space activate, mouse
selects, and Escape cancels. Ctrl+G retains its existing global behavior.
Cross-folder targets produce a typed, effect-free refusal with a bounded plain notice. No resume action is offered or executed; clients must not render an action picker. Rendering the notice performs no claim, save, child settlement, or process spawn. Action-bearing legacy clients may ignore the refusal data and continue safely.

```json
{"type":"list_sessions","id":"ls-1","scope":"global"}
```

---

### `search_session_metadata`

Search saved-session metadata: title, exact opaque key, repository label and
execution path. Owner: `SearchSessionMetadata` (#2010). Transcript content is
never matched — no transcript is read to MATCH. Freshness alone decides what is
read: a record version already indexed — or already rejected by this process
(a verdict on its bytes; a failed read is never remembered, and no rejection
is persisted) — costs a `stat`;
a new or changed record is read once per validating half, and an absent,
unreadable or version-incompatible index is rebuilt by reading every record
(twice in all) on the first search. An answer is not authorization to restore
a row.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"search_session_metadata"` | yes | |
| `id` | string | no | Correlation ID |
| `query` | string | yes | Literal text, never a pattern. Compared as visible text (controls and invisible format characters dropped, whitespace collapsed, then case folded — Unicode lower-casing plus final sigma = sigma, `ß` = `ss`, dotted/dotless `i` = `i`; no normalization, no ligature expansion); whitespace separates terms and every term must occur in the title, repository label or path (in `local` scope: the title, or the path **below the workspace group's root** — the root and the label are every local row's, so they are not matched); the whole trimmed query must equal a key byte for byte to match it. Nothing visible names every session in scope. More than 256 visible characters is refused whole — counted as typed, before the fold: `ß` is one character though it is searched as `ss`. A missing or non-string `query` is an uncorrelated `parse_error` |
| `scope` | `"local"` or `"global"` | no | Defaults to local; unsupported values are rejected |
| `generation` | integer | no | The client's own counter, echoed so a client can discard an answer it no longer wants. Defaults to 0. It must be a JSON **integer from 0 to 18446744073709551615** (2^64−1) and is echoed exactly — never rounded, so an echo equal to what was sent is the answer to that request. Anything else — a fraction (`7.9`, `7.0`), a negative number, `-0`, an integer ≥ 2^64, a number no `f64` holds (`1e400`), a string, an array — is a **correlated** refusal (`refused: "generation must be an integer from 0 to 18446744073709551615"`, `generation` echoed as 0), never an uncorrelated `parse_error`. Send one ≤ 2^53 if your own JSON reader cannot round-trip larger integers |
| `limit` | number | no | Rows returned at most: 1–500, default 200. Any JSON number is accepted and clamped into range (negative or 0 → 1, a fraction truncated, anything larger → 500 — a literal no `f64` holds, `1e400`, included) and the effective value echoed. Anything that is not a number is a **correlated** refusal (`refused: "limit must be a number"`, the readable `generation` still echoed) |

**Response data:**

```json
{
  "query": "flaky",
  "scope": "global",
  "generation": 7,
  "limit": 200,
  "sessions": [
    {"key":"chat-1765930000-1a2b3c","title":"Fix the flaky test","messageCount":12,"updatedUnixSecs":1765930000,"updatedAt":1765930000,"homeState":"scoped","executionPath":"/work/project","resumeEligible":false,"homeVersion":"h1-0123456789abcdef","repositoryLabel":"project","matched":["title"]}
  ],
  "totalMatches": 1,
  "searched": 5201,
  "truncated": false,
  "refused": null,
  "diagnostics": [],
  "diagnosticsTotal": 0,
  "rebuilt": false
}
```

A row is a [`list_sessions`](#list_sessions) row — same fields, same
identity-bound `homeVersion`, to be echoed as `resume_session.expectedHomeVersion`
— plus:

| Row field | Type | Description |
|---|---|---|
| `repositoryLabel` | string \| null | Safely rendered name of the repository (the directory holding the Git common dir; shared by linked worktrees) or folder; `null` for a legacy unscoped or unavailable home |
| `matched` | string[] | Which metadata matched, in rank order: `key`, `title`, `repository`, `path`; empty when the query named every session |

| Answer field | Type | Description |
|---|---|---|
| `query` | string | The visible text of the query — exactly what was searched, before the case fold (invisible and control characters dropped, whitespace collapsed) — safely rendered; of a refused over-long query, its first 256 characters |
| `sessions` | row[] | Best first: best matched field (key, title, repository, path), then newest, then key |
| `totalMatches` | integer | Matches in scope before `limit` |
| `searched` | integer | Sessions in scope the query was matched against |
| `truncated` | boolean | `totalMatches` exceeds the rows returned |
| `refused` | string \| null | Why nothing was searched (`query too long: …`, `limit must be a number`, `generation must be an integer from 0 to 18446744073709551615`); the rows are then empty |
| `diagnostics`, `rebuilt` | | As for `list_sessions`: every search validates the derived index against authority; a malformed record (`cli_x.json: session record unavailable: …`) and a sidecar that needs repair (`cli_x.home: home needs repair: …`) are each named by file, once per answer, and hide no sibling. Bounded as for `list_sessions`: the first 20 lines, then `… and N more`; `diagnosticsTotal` counts them all |

`homeState` is `legacy_unscoped` for a session saved before folders were
tracked: it is found by title and key, and a client labels it explicitly (the
TUI shows "No folder recorded (older session)"). Local scope without current
workspace facts searches nothing and says why in `diagnostics`. An
`executionPath` or `repositoryLabel` that is not UTF-8 spells each byte that is
no text as `\xNN` (`/work/caf\xE9`) and doubles a literal backslash (a folder
really named `caf\xE9` is `caf\\xE9`), so the spelling is injective: two
different folders never share one, and a query matches what is shown. Every
`executionPath` on the wire is spelled this way — a `list_sessions` row, a
search row and the `resume_session` decision name one folder with one text.

**Cost and ordering.** Every search stamps every saved record (about 90 ms on
a 5,200-record store; the 50 ms target is missed) and folds every path, so the
cost is linear in the stored text: 3,600 homes under a 3,900-character
non-ASCII path took a warm global search from 94 to about 320 ms (#2042). Commands are answered FIFO:
searches queued back-to-back delay a later `get_state` or `abort` on the same
connection by their sum. Send one search at a time and the latest text when it
is answered, as the TUI does.

The TUI's `/resume` search box sends this command for the scope on screen as the
text changes — one search in flight per picker, the latest text sent when the
answer arrives (two searches for a burst typed faster than a round trip, one per
key otherwise). Only the answer that carries the id it sent and its latest
`generation` settles the picker, and only settled rows are acted on; an
overtaken answer is shown as progress under `Searching…`, never across a scope
change or a cleared box; an unanswered search is re-issued once after 5 s. An
empty box lists the scope with `list_sessions`. The picker decides no scope and
matches nothing itself — except against a harness that predates this command
(an uncorrelated `parse_error: unknown variant`), where it says so once and
filters the listed rows locally by the same visible-text rule. A selected row
goes to `resume_session` with its `homeVersion` like any listed row.

```json
{"type":"search_session_metadata","id":"ss-1","query":"flaky","scope":"global","generation":7}
```

---

### `resume_session`

Switch the active UDS conversation to a persisted session. The current session is saved first. Owner: `ResumeSavedSession` (#1863, #1968).

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"resume_session"` | yes | |
| `id` | string | no | Correlation ID |
| `session` | string | yes | The target, resolved exactly (never by prefix or fuzzy match, and never through the discovery index): a full `chat-…` key (as listed), an already-qualified `cli:<name>` key, or a bare CLI session name such as `default` or `work` (resolved to `cli:<name>`) |
| `action` | string | no | One explicit action of a resume decision (#2011): `open_original`, `fork_current`, `locate`, `associate` or `cancel`. Absent — or `null`, which is the same as absent, as for every optional field — = restore. Any other spelling, and any non-string, is a `parse_error`; like every `parse_error` it is **uncorrelated** (no `id`: the line never decoded into a command), so a client must never await its `id` for a spelling this list lacks |
| `expectedHomeVersion` | string | with an `action` other than `cancel` | The `homeVersion` the client was shown for **this session** (its `list_sessions` row or its decision). When present it must still be the authority's version (for a restore it is re-checked under the target's claim); a stale token, another session's token or a never-issued one is refused with `stale_home_version`. An explicit action acts on the authority the client saw: without the field every action but `cancel` on an existing session is refused with `home_version_required` before any effect (a missing session is `not_found` first — see the action order below). Optional for a restore (`/resume <key>` as typed). A non-string is an uncorrelated `parse_error` |

**Behavior:**
- **Agent idle:** Settles delegated children, saves the current session, claims and loads the target, releases the departing key, resets the effort override, restores the target's workflow run (or resets the engine when it saved none), switches subsequent prompts to that history, emits `ledger_advanced` with a new `epoch`, and answers `{"outcome":"resumed","session":"<name>","sessionKey":"<key>","messageCount":N}`. `get_state.sessionKey` and `get_session_stats.sessionKey` follow the target; usage statistics restart at zero for the resumed session
- **Agent running:** turns and command dispatch run on one task, so a request sent mid-turn is **queued and executed after the turn ends** — the user who asks for a resume mid-turn is switched when the turn finishes, not refused; nothing answers until then. The typed refusal `{"outcome":"refused","code":"busy"}` (error `"cannot resume a session while agent is running"`, nothing settled, saved or claimed) is a **defensive guard that is unreachable** while commands and turns share that one dispatch task: a client should map the code, and should not expect to observe it
- Invalid targets are rejected with `"session name must contain only alphanumeric, '-', or '_'"` (the same rules as `quecto agent --session`; no other prefix is admitted)
- Missing sessions return `success: false` with `"session not found: <name>"` (`not_found`) — known from the store's existence check **before any effect**, for a restore and for every action but `cancel`, whatever token the request carries: no child is settled, nothing is saved and nothing is claimed for a mistyped key (for a restore the loop's own key is excepted: the departing save may be what first writes it, so its absence is reported under the claim; an action saves nothing, so it has no such exception). A key another live harness holds open is refused with the ownership error. Every refusal keeps the current conversation in place, and a failure under the claim releases that claim unless it is the loop's own key, which is never released (#1995)
- Ephemeral loops (`--no-session`) refuse with `"cannot resume sessions in ephemeral mode"`
- **Typed answers (#2011):** every answer to a decoded `resume_session` carries `data.outcome` (a line that does not decode is the uncorrelated `parse_error`, which has no `data`). Restored: `success:true`, `{"outcome":"resumed","session","sessionKey","messageCount"}`. Cancelled (`action:"cancel"`): `success:true`, `{"outcome":"cancelled","session"}` — nothing is settled, saved, claimed or restored, and the session is not looked up. **Mixed-version caveat:** like every response on the multi-client server this answer is broadcast (the transport has no requester-only path), and a client that predates #2011 reads any `resume_session` success as a restore — such a peer connected to the same harness shows a false "Resumed" notice and refreshes; it adopts no key (the payload has no `sessionKey`) and no state changes. Clients from #2011 on ignore `cancelled`. Refused: `success:false`, `error` text plus `{"outcome":"refused","code"}` with a stable `code` — `busy`, `ephemeral`, `invalid_name`, `not_found`, `stale_home_version`, `current_scope_unavailable` (this runtime's own execution directory cannot be discovered: nothing can be admitted, so this is a refusal and never a decision), `home_version_required` (adds `action`), `action_not_offered` (adds `action`), `action_unavailable` (adds `action`, `reason`), `action_executed_elsewhere` (adds `action`), `transition_refused`, `save_failed`, `claim_refused`, `load_failed`
- **Resume decisions (#2009, #2011):** only a session saved in this same execution directory restores. Any other existing target is answered — when this runtime's own execution directory is discoverable; otherwise every target is refused `current_scope_unavailable` — `success:false` with `error` beginning `session resume unavailable:` and the typed decision `{"outcome":"decision","code":"decision_required","session","sessionKey","kind","homeVersion","executionPath"|null,"detail"|null,"actions":[{"action","available","reason"|null}]}`. `kind` and its offered actions, in order: `cross_folder` (another execution directory, a grouped worktree included) → `open_original`, `fork_current`, `cancel`; `home_missing` (the saved directory is missing, moved or inaccessible), `home_changed` (same directory, different workspace group) and `home_unknown` (home metadata present but uninterpretable; `executionPath` is `null`) → `locate`, `fork_current`, `cancel`; `legacy_unscoped` (no home was ever recorded) → `associate`, `cancel`. `available` is the harness's statement that an executor for the action is composed; until the open-original, fork and locate/associate executors are delivered only `cancel` is available and every other offer carries the `reason` (text for people: what to do instead, no tracker numbers; the machine-readable facts are `available` and the `action_unavailable` code). An explicit action other than `cancel` is answered effect-free in one fixed order and is never replaced by a restore or by another action: the session exists (`not_found`) → the request names a version (`home_version_required`) → the version is this session's current one (`stale_home_version`) → the session's decision kind offers the action (`action_not_offered` — also the answer for any action on a session that simply restores here) → an executor is composed (`action_unavailable`; `action_executed_elsewhere` if one is). The decision is made twice: by an effect-free pre-flight (so a decision settles no child, saves nothing and claims nothing) and again under the target's claim, which is then released; the current conversation, identity and ownership are preserved. `executionPath`, `detail`, `reason`, `session` and `error` are untrusted metadata rendered bounded (4096 characters) with every control character **and every invisible character that reorders, hides or splits text** — U+00AD, U+034F, U+061C, U+180B–U+180F, U+200B–U+200F, U+2028–U+202E (line/paragraph separators and the bidi embeddings/overrides), U+2060–U+206F (word joiner, invisible operators, bidi isolates), U+FEFF, U+FFF9–U+FFFB and the tag characters U+E0000–U+E007F — replaced by U+FFFD, so no socket client receives Trojan-Source material. Variation selectors (U+FE00–U+FE0F, U+E0100–U+E01EF) are deliberately kept: they select how a visible glyph is drawn (emoji presentation, ideographic variants) and conceal nothing; combining marks are likewise left to the presenting client to bound. The startup open of the loop's own session applies the same rule, worded for the command line: `session '<key>' cannot start here: ...` names the composed key and the way out (`-s <name>`, `--no-session`; the transcript stays visible in the Global list; explicit association is #2014) and exits 1 — see `sessions.md`
- Resuming the key the loop already stands for reloads it from disk in place: the key is unchanged (on success no departing key is released), the history and workflow run are restored from the record, and usage statistics restart; a failure on that path keeps the loop's own claim (#1995)
- **Subagents (#1937):** the transcript, workflow run and past child messages are restored; the operational child roster is reset and **no child row is created from persisted records**. Persisted rows are history only — no socket is probed, no pid compared, no child row re-created or monitored, whatever the row's recorded liveness — because a launcher-created child cannot outlive the harness that launched it. Re-spawn the workers you need; each gets a fresh identity and launch generation

**Example:**

```json
{"type":"resume_session","id":"rs-1","session":"work"}
{"type":"resume_session","id":"rs-2","session":"chat-1750000000-ab","expectedHomeVersion":"h1-0123456789abcdef"}
{"type":"resume_session","id":"rs-3","session":"chat-1750000000-ab","action":"cancel"}
{"type":"resume_session","id":"rs-4","session":"chat-1750000000-ab","action":"fork_current","expectedHomeVersion":"h1-0123456789abcdef"}
```

---

> Descendant queries (`get_state`/`get_report` with `agent_id`) are forwarded
> concurrently: their replies may arrive before earlier queued commands, and at
> most eight are in flight per process; beyond that the query is answered with
> an explicit capacity error rather than queued.

### `get_state`

Return the slim live supervision projection for the active session. This is the
command to use for occasional in-flight state/progress checks; transcript and
history inspection remain the job of `get_messages`.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"get_state"` | yes | |
| `id` | string | no | Correlation ID |
| `since` | integer | no | Previously observed `generation`; when unchanged, return only the bounded unchanged marker |

**Response data (changed or no `since`):**

```json
{
  "state": "runningTool",
  "effort": "low",
  "model": "anthropic/claude-sonnet-4-6",
  "progress": {
    "state": "advancing",
    "reason": "4 tools completed in the last 120 seconds"
  },
  "generation": 27,
  "workflow": {
    "activeTemplate": {
      "id": "bugfix"
    },
    "currentStep": {
      "index": 2,
      "key": "red",
      "label": "Reproduce the bug in a failing test",
      "phase": "RED",
      "done": false
    }
  }
}
```

When no workflow template is selected, the `workflow` field is omitted entirely.

**Response data (unchanged):**

```json
{"unchanged": true, "generation": 27}
```

| Field | Type | Description |
|---|---|---|
| `state` | string | Current model/execution state such as `idle`, `streaming`, `thinking`, `runningTool`, or `finalizing` |
| `effort` | string \| null | Effective session effort (`null` means provider default / unset) |
| `model` | string | Active model (qualified `provider/model` or bare name) |
| `progress` | object | Evidence-based progress verdict with only `state` and `reason` |
| `generation` | integer | Activity cursor for `since` comparisons |
| `workflow` | object \| omitted | Slim selected-workflow identity and current step only |
| `admission` | object \| omitted | Bounded inference-admission view (#1679); present only when the process joined an admission authority |

**Admission (`admission`, #1679 P4).** When the process shares an inference
authority ([inference-admission.md](inference-admission.md)) the projection
carries its own attempts' admission activity beside the phase. Admission is
never a `state` value: a process whose next attempt is queued at the authority
stays in `thinking` (or whatever phase it is in) and its `progress` becomes
`waiting`, which supervisors must treat as neither idle nor stalled.

```json
{
  "state": "thinking",
  "progress": {
    "state": "waiting",
    "reason": "2 inference attempts waiting for admission in group anthropic for 12s; cooldown 30s remaining"
  },
  "admission": {
    "waiting": 2,
    "admitted": 0,
    "longestWaitSeconds": 12,
    "groups": [
      {"group": "anthropic", "cooldown": {"state": "until", "remainingSeconds": 30}}
    ],
    "counters": {"completed": 7, "refused": 0, "cancelled": 1, "abandoned": 0},
    "hidden": 0,
    "revision": 41,
    "directory": "/home/me/.quecto/admission",
    "epoch": 1,
    "connected": true,
    "authorityStatus": "connected"
  }
}
```

| Field | Type | Description |
|---|---|---|
| `waiting` / `admitted` | integer | Live attempts of this process queued at / granted by the authority (exact counts) |
| `longestWaitSeconds` | integer \| omitted | Longest sampled wait; omitted when nothing waits or every waiting attempt is beyond the sample (`hidden`), never `0` for "unknown" |
| `groups[].group` | string | Quota group |
| `groups[].cooldown` | object \| omitted | `state` is `until` (with `remainingSeconds`), `unknown` (throttled without a deadline) or `unavailable` (authority marked the group unavailable) |
| `groups[].lastRefusal` | string \| omitted | Most recent refusal reason, bounded to 200 bytes |
| `counters` | object | Lifetime `completed`, `refused`, `cancelled` (wait given up, e.g. `abort`) and `abandoned` (permit dropped without completion) |
| `hidden` | integer | Live attempts beyond the 64-attempt sample that `longestWaitSeconds` is derived from; when non-zero the longest wait may be under-reported |
| `revision` | integer | Advances on every admission transition (queued, granted, completed, refused, cancelled, abandoned, cooldown learned); time-derived values are not transitions |
| `directory` | string \| omitted | The authority directory this process is bound to (#2024 S3); not forwarded for a descendant |
| `epoch` | integer \| omitted | The authority epoch this process's *current* capability was minted in; a root re-registers into the current epoch after a broker restart or reset, so it moves |
| `connected` | boolean \| omitted | Whether the link is open *and* holds a live capability (a revoked root reads `false` until it re-registers) |
| `authorityStatus` | string \| omitted | `connected`; `reconnecting` (the link lost its broker or its capability and this root will re-register on its next attempt); `unavailable` (a child whose parent-minted capability is gone — its parent respawns it — or a root whose bounded reconnection was exhausted, or whose broker came back with a different policy: restart required); presentation only |

`directory`, `epoch`, `connected` and `authorityStatus` describe the shared
authority itself (#2024 S3, one host-wide broker). They are read from the live
link on every `get_state` (never captured at attach) whenever the process is
bound to an authority, even with no live attempts, and ride on every pushed
`admission_state_changed` of a bound process — which is also emitted, with the
activity unchanged, whenever the link's health changes (loss, revocation,
reconnection, exhaustion), so a client's badge is live without polling. The
TUI footer renders `authorityStatus` as `admission ✓` / `⟳` / `✗`.

The `waiting` verdict takes precedence over the tool-window verdicts
(`advancing`, `active`, `quiet`) while any attempt is queued. A changed
admission `revision` advances `generation` once per observation (several
transitions between two polls advance it once); the revision moves in the
same critical section as the view, so a `since` poll never returns the
unchanged marker across a transition (ADR-0024 freshness and delta bounds). `revision` and `generation` track transitions only: elapsed waits,
`remainingSeconds` and the `reason` text are re-derived on every full read and
keep moving while a `since` poll answers `unchanged`; poll without `since` to
watch a wait grow or a cooldown run out. `abort` while an attempt waits
cancels the wait at the authority; the next `get_state` no longer reports
`waiting` and counts the wait under `counters.cancelled`.

`get_state` intentionally does not include static vocabularies, transcript
counts, sync/history state, context-window metadata, available workflow
templates, full workflow step lists, guidance, or detailed execution payloads.
Busy `get_state` snapshots use the same slim shape and do not carry a snapshot
marker. For an exact `since` query, an id-less connect-time snapshot is accepted
only when its generation equals `since`, and is then finalized as the unchanged
marker above. A snapshot newer than `since` is only a point-in-time observation;
it is not used as changed-data proof for that cursor, so the caller waits for the
correlated live response instead.


### `get_messages`

Return the stable committed conversation transcript as bounded pages (#1061).
Use this for full/end-of-turn output inspection; use `get_state` for live
in-flight supervision. While a turn is active, a busy snapshot may lag the
mutable in-flight conversation and is marked `snapshot: true`.

This UDS protocol command remains a history paging primitive: omit `count` and
`before` for the newest page (up to the protocol page size of 64 messages);
pass `count: N` for the last N messages; pass `before: <messageId>` (a message
id from a prior response's `before` field) to fetch the adjacent older page.

The model-facing `agent_cmd get_messages` tool layers report semantics on top:
omitting/nulling both `count` and `before` requests the parent-scoped unread
report and may advance that parent's report cursor only after delivery;
providing either field selects this cursor-neutral history paging behavior.
Without an explicit `count`, history is never returned unbounded — walk
`before` cursors until `hasMoreBefore` is `false` to reach the beginning of
the session. An explicit `count` keeps the legacy last-N contract (it may
exceed one page); every response line is still byte-capped on the wire, with a
cursor advertised for anything the cap removes.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"get_messages"` | yes | |
| `id` | string | no | Correlation ID |
| `count` | integer | no | Maximum number of trailing messages to return for the UDS history primitive; in model-facing `agent_cmd`, omit/null with no `before` for the unread report |
| `before` | string | no | Paging cursor: return messages strictly before this message id. An unknown/stale cursor is an error, not a silent restart |

**Response data:**

```json
{
  "messages": [
    {
      "id": "8f14e45f-ceea-4670-9f5c-2f1a72f1a72f",
      "role": "user",
      "content": "Hello",
      "toolCalls": [],
      "toolCallId": null,
      "toolName": null,
      "isError": false,
      "collapsed": false
    },
    {
      "id": "9b74e45f-ceea-4670-9f5c-2f1a72f1a730",
      "role": "assistant",
      "content": "Hi! How can I help?",
      "toolCalls": [],
      "toolCallId": null,
      "toolName": null,
      "isError": false,
      "collapsed": false
    }
  ],
  "before": "8f14e45f-ceea-4670-9f5c-2f1a72f1a72f",
  "hasMoreBefore": true
}
```

Page metadata:

| Field | Type | Description |
|---|---|---|
| `before` | string \| null | Cursor for the adjacent older page (the oldest message included in this page); `null` when the beginning of history is reached |
| `hasMoreBefore` | boolean | Whether older history exists before this page. Legacy corner: an explicit `count: 0` returns an empty page reporting `hasMoreBefore: false` with no cursor (an empty window has no oldest-included message to anchor one) |

To page back to the beginning of the session (`request` = send the command,
then read events until the `response` whose `id` matches):

```python
resp = request(sock, {"type": "get_messages", "id": "page-0"})
while resp["data"]["hasMoreBefore"]:
    resp = request(sock, {
        "type": "get_messages",
        "id": "page-" + resp["data"]["before"],
        "before": resp["data"]["before"],
    })
```

Each message contains:

| Field | Type | Description |
|---|---|---|
| `id` | string | Stable message id (#1060) — usable as a `before` cursor, a `get_message` lookup key, or a `rewind_to` target |
| `role` | `"system"` \| `"user"` \| `"assistant"` \| `"tool"` | Message author |
| `content` | string | Message text (a ladder-demoted stub when `collapsed` is true) |
| `toolCalls` | array | Tool calls made by the assistant (each with `id`, `name`, `arguments`) |
| `toolCallId` | string \| null | For `tool` messages: which tool call this is a result for |
| `toolName` | string \| null | For `tool` messages: name of the tool that produced this result |
| `isError` | boolean | Whether a `tool` message carries an error result |
| `collapsed` | boolean | `true` when the context ladder demoted this message to a stub — recall the full body on demand via `get_message` with this `id` (#1061) |
| `thinking` | array | Optional assistant-only display-safe thinking blocks, omitted when absent. Text blocks use `{ "kind": "text", "text": "..." }`; redacted/private provider blocks use `{ "kind": "redacted" }` and never expose signatures, encrypted reasoning, or redacted payload bytes. `content` remains answer-only. |

---

### `get_message`

Return one stable message by id. For oversized content, clients pass byte-range
fields and walk the response cursor until `hasMoreContent` is `false`; every
ranged response is capped to fit the UDS frame limit (#1094).

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"get_message"` | yes | |
| `id` | string | no | Correlation ID |
| `messageId` | string | yes | Stable message id from `get_messages`, `turn_end.messageRefs`, or `agent_end.messageRefs` |
| `offset` | integer | no | Content byte offset to start the returned range; omit only when the caller knows the full message fits in one frame |
| `limit` | integer | no | Requested maximum content bytes for this page; the server may return fewer bytes to preserve the frame cap |
| `agent_id` | string | no | Forward the lookup to a spawned child agent |
| `toolCallId` | string | no | When set, recover that tool call's arguments instead of the message body |

**Response data:** the message fields above plus range metadata when `offset` or
`limit` is present:

| Field | Type | Description |
|---|---|---|
| `content` | string | Returned content slice for this page |
| `offset` | integer | Byte offset of `content` in the full message |
| `nextOffset` | integer | Offset to request next; equals `contentLength` on the final page |
| `contentLength` | integer | Full message content length in bytes |
| `hasMoreContent` | boolean | `true` when the client should request another page using `nextOffset` |

Example page walk:

```json
{"type":"get_message","id":"m-page-0","messageId":"...","offset":0,"limit":65536}
```

```json
{"content":"...","offset":0,"nextOffset":65536,"contentLength":131072,"hasMoreContent":true}
```

---

### `sync`

Reconcile a client's transcript with the agent's committed ledger. Answered on both transports — the idle dispatch loop and, while the agent is busy, the per-connection reader — through the same owner: `SynchronizeTranscript` (#1857, #1968).

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"sync"` | yes | |
| `id` | string | no | Correlation ID |
| `epoch` | integer | yes | The ledger epoch the client last synchronised against (`0` for none) |
| `sinceRev` | integer | yes | The last committed revision the client holds |
| `agent_id` | string | no | Forward the sync to a spawned child agent |

**Response data:** a delta when the client's epoch is current and `sinceRev` is at or above the oldest retained revision, otherwise a resync page.

| Field | Type | Description |
|---|---|---|
| `epoch` | integer | The current ledger epoch (advances on `clear_history`, `rewind_to`, `new_session`, `resume_session`) |
| `rev` | integer | The newest committed revision |
| `resync` | boolean | `true` when the client must replace its transcript with the carried page |
| `messages` | array | Delta: the committed messages after `sinceRev`, in commit order, as many as fit one frame. Resync: the newest history page (with `before` / `hasMoreBefore` as `get_messages`) |
| `nextRev` | integer \| null | Delta: the revision to continue from when the delta was cut at the frame budget; `null` when caught up |
| `caughtUp` | boolean | Delta: `true` when nothing was left out |

Clients continue a cut delta with `sinceRev = nextRev`. The `ledger_advanced` event (`{"type":"ledger_advanced","epoch":E,"rev":R}`) tells a client when a sync is worth sending.

---

### `get_report`

Return the latest assistant report of the session and, on request, write a retained raw export. Owner: `ExportSessionReport` (#1859, #1968). Available while busy through the reader task.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"get_report"` | yes | |
| `id` | string | no | Correlation ID |
| `export_raw` | boolean | no | Also export every retained message and spill entry under `<base>/artifacts/session-exports/session-*/` with a checksum manifest |
| `agent_id` | string | no | Forward the request to a spawned child agent |

**Response data:** `{"report":{"messageId","content","contentTruncated","fullLengthBytes"},"recovery":{"command":"get_message","messageId","offset"}|null,"snapshot":true}`, or `{"report":null,"snapshot":true}` when no substantive assistant message exists; with `export_raw`, `rawExport: {"path","manifest","sha256","bytes","scope":"retained_snapshot"}`. Refusals: `"session export directory unavailable"`, `"two raw exports are already running; retry after completion"`, `"session changed during export; retry against the new epoch"`.

---

### `get_session_stats`

Return token usage and cost statistics for the current session. `sessionKey` is the active session's identity; the usage counters restart at zero whenever `new_session` or `resume_session` moves the loop to a different key, and on `clear_history` / `rewind_to`.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"get_session_stats"` | yes | |
| `id` | string | no | Correlation ID |

**Response data:**

```json
{
  "sessionKey": "cli:default",
  "userMessages": 3,
  "assistantMessages": 3,
  "toolCalls": 2,
  "toolResults": 2,
  "totalMessages": 10,
  "tokens": {
    "input": 0,
    "output": 0,
    "cacheRead": 0,
    "cacheWrite": 0,
    "total": 0
  },
  "costMicroUsd": 0,
  "cacheHitRatio": null,
  "contextTokens": 0,
  "maxContextTokens": 0
}
```

| Field | Type | Description |
|---|---|---|
| `sessionKey` | string | Session identifier |
| `userMessages` | integer | Number of user messages |
| `assistantMessages` | integer | Number of assistant messages |
| `toolCalls` | integer | Number of tool calls made |
| `toolResults` | integer | Number of tool results received |
| `totalMessages` | integer | Total messages (including system, tool) |
| `tokens` | object | Normalized token usage breakdown (`input` full-price input, `output`, `cacheRead`, `cacheWrite`, `total` = input + output) |
| `costMicroUsd` | integer | Estimated cumulative cost in micro-USD |
| `cacheHitRatio` | number or null | Shared cache-hit ratio (`cacheRead / (input + cacheRead + cacheWrite)`) or null when denominator is zero |
| `contextTokens` | integer | Provider-reported prompt occupancy when available, else local estimate |
| `maxContextTokens` | integer | Active model context-window limit (`0` when unknown) |

---

### `set_model`

Switch the active model at runtime. The new model takes effect on the next prompt.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"set_model"` | yes | |
| `id` | string | no | Correlation ID |
| `model` | string | option A | Qualified model name, e.g. `"anthropic/claude-sonnet-4-6"` |
| `provider` | string | option B | Provider name (used with `modelId`) |
| `modelId` | string | option B | Model ID within the provider |
| `persist` | `"local"` \| `"global"` | no | Also record the model as the configured default of that layer (#2024 S2): `"local"` writes `agents.defaults.model` in the repository overlay `<cwd>/.quecto/config.json` (created if absent, trust recorded), `"global"` in the run's global layer — `<base_dir>/config.json`, or the `--config` file when the run was started with one. Absent: the switch is in-memory, as before. |

You must provide either `model` OR both `provider` + `modelId`. Providing neither (or empty strings) returns an error.

**Persisting a default.** The record goes through the same safe writer as `quecto config set` (exclusive hold, the file and the merged configuration validated before a byte is written, only the addressed key changed, `providers`/`admission` never touched) and happens *before* the switch is applied, so a refused record leaves the session unchanged: `success: false` with the writer's reason and remedy (an untrusted overlay → `run \`quecto config trust\` first`; a symbolic link at the overlay; a run started with an explicit `--config`, which has no overlay; a run without a reloadable configuration). Only a qualified `provider/model` id whose provider the published catalogue lists models for is recorded — a bare id (`select it as provider/model`) and a provider the published catalogue lists no models for (`the published catalogue lists no models for \`X\``: unconfigured, so every later start there would fail its first prompt, or configured but not yet refreshed — a failed source, a custom endpoint before its first `refresh_models`) are refused for persistence, while an id the catalogue does not enumerate on a listed provider, or one that is not runnable *now* (missing credential), is recorded as the switch is applied — and the id recorded is the qualified one the switch resolved, so `provider` + `modelId` persists as `provider/modelId`. Because an unlisted id on a listed provider *is* recorded, the reply's `selection.status` is the typo check when persisting: `unknown_model` there means the catalogue does not enumerate the id you just pinned, so check the spelling (`quecto config unset` rolls it back). Any other `persist` value is an error before anything happens. Two consequences worth knowing: the record is a configuration edit, so the poll before this session's next prompt rebuilds the provider runtime from the files and re-applies the persisted tool policy, which clears live-only `set_tool_policy` overlays exactly as an external `quecto config set` would; and the switch still resets this session's effort for the new model (`low` where accepted) while a previously persisted `agents.defaults.effort` stays as it is — a later start admits that pair through the startup rule (an effort the model does not accept is dropped with a warning). If recording trust fails *after* an overlay write (`wrote … but could not record it as trusted`), the file holds the new default but is untrusted until `quecto config trust` — the one case where a refusal is not "nothing changed". A persisted default is read at startup: every *new* agent started in that directory (local) or anywhere (global) starts on it; other running sessions keep the model they have (a reload rebuilds providers, it does not re-read the default model).

**Model routing:**
- **Qualified names** (`provider/model`): Routed to the matching provider. If no provider matches the prefix, prompts will fail with `"no configured provider matches model prefix 'X'"` — but the agent stays alive and you can switch to a valid model
- **Bare names** (`model`): Sent to the first configured provider, which may not support the model

**Response:** `success: true` on valid input, `success: false` with an error message on validation failure. With `persist`, the data carries where the default landed beside the selection verdict:

```json
{"selection":{"status":"ok","provider":"openai-api","generation":3},"persisted":{"scope":"local","path":"/work/app/.quecto/config.json"}}
```

> **Important:** `set_model` only swaps a string — it performs no validation against the provider. Errors surface on the next `prompt`.

**Examples:**

```json
{"type":"set_model","id":"sm-1","model":"anthropic/claude-sonnet-4-6"}
```

```json
{"type":"set_model","id":"sm-2","provider":"anthropic","modelId":"claude-sonnet-4-6"}
```

```json
{"type":"set_model","id":"sm-3","model":"openai-api/gpt-5.6-luna","persist":"local"}
```

---

### `set_effort`

Switch the session reasoning-effort level at runtime (#1067). Applied to every subsequent turn. Validated against the active model's provider vocabulary (OpenAI reasoning models use `none`–`xhigh`; Anthropic 4.6 uses `low`/`medium`/`high`/`max`).

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"set_effort"` | yes | |
| `id` | string | no | Correlation ID |
| `effort` | string | yes | Effort level string |
| `persist` | `"local"` \| `"global"` | no | Also record the level as `agents.defaults.effort` of that configuration layer (#2024 S2), with the same writer, ordering and refusals as `set_model`'s `persist`. The level is validated against the active model first; a refused record leaves the session unchanged. |

**Response data (success):**

```json
{"effort": "high"}
```

With `persist`: `{"effort":"high","persisted":{"scope":"global","path":"/home/u/.quecto/config.json"}}`.

**Error:** `success: false` with a message listing the valid levels for the active model, or the writer's reason when the record was refused.

**Example:**

```json
{"type":"set_effort","id":"se-1","effort":"xhigh"}
```

---

### `list_models`

Return configured and built-in models from the runtime registry (`models.json` + built-ins).

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"list_models"` | yes | |
| `id` | string | no | Correlation ID |

---

### `new_session`

Switch to a fresh user-chat session. The previous session is saved first. Rejected while the agent is streaming. Owner: `StartFreshConversation` (#1862, #1968).

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"new_session"` | yes | |
| `id` | string | no | Correlation ID |

**Behavior:**
- **Agent idle:** Settles delegated children, saves the departing session, replaces the roster, clears the conversation (system prompt kept), draws a fresh `chat-<unix-secs>-<uniq>` key (never claimed until its first save), releases the departing key, resets the effort override and the workflow engine, clears the fresh key's retention namespace, emits `ledger_advanced` with a new `epoch`, and answers `{"sessionKey":"chat-…"}`. `get_state.sessionKey` follows it and usage statistics restart at zero
- **Agent running:** Returns `success: false` with error `"cannot start a new session while agent is running"`
- **Children cannot be settled:** `success: false` with the settlement refusal (`…could not be settled; the current session was kept`, `subagent teardown was interrupted…`, `…no fleet teardown is available…`, `…remain after the teardown…`); the current session is kept
- **Save failure:** `success: false` with `"failed to save current session: <store error>"`; the current session is kept

---

### `persist_session`

Save the current session now, without waiting for the next turn boundary. Owner: `SaveSession` (#1860, #1968) — the same transaction every other save of the loop requests.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"persist_session"` | yes | |
| `id` | string | no | Correlation ID |
| `restoreReason` | string | no | `"ordinary_tui_exit_stopped"` marks a TUI's ordinary exit (#1586): the persisted sub-agent roster is emptied and the reason is stamped on the record. Any other value, or none, records the legacy unspecified reason |

**Response:** the correlated `response` with `success: true`, or `success: false` carrying the store's error text verbatim.

---

### `rewind_to`

Rewind conversation history to a selected user-message boundary. Prefer stable `messageId` (from paged history). `messageIndex` is retained only for single-page conversations; beyond one history page it is rejected rather than misapplied. Owner: `RewindConversation` (#1865, #1968).

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"rewind_to"` | yes | |
| `id` | string | no | Correlation ID |
| `messageId` | string | preferred | Stable message id to rewind to; when both are given, `messageId` wins |
| `messageIndex` | integer | legacy | Page-local index — only honoured while history fits one page |

**Behavior:**
- **Agent idle:** Truncates the conversation at the selected user message (that message is removed too), strips retention residue, resets the ledger to the survivors, clears the retention namespace, saves, emits `ledger_advanced` with a new `epoch`, and returns `success: true`
- **Agent running:** `"cannot rewind while agent is running"`
- Neither field: `"rewind requires messageId or messageIndex"`; unknown id or out-of-range index: `"rewind target not found"`; an index on a conversation longer than one history page: `"messageIndex is ambiguous beyond one history page; rewind requires messageId"`; a target that is not a user message: `"invalid rewind target"`. Every refusal leaves the conversation untouched
- **Save failure:** the rewind is applied and `ledger_advanced` emitted; the response is `success: false` with `"failed to save rewound session: <store error>"`

---

### `reload`

Force a runtime config reload (provider/model registry plus config watch surfaces): the reload-runtime-configuration use case (#1849) rebuilds the provider runtime from the run's config file and `models.json` regardless of whether they changed, from one parse of the config. Reload also reapplies `tools.policy.entries` from that same read as the persisted tool-policy baseline and clears live-only AgentLoop tool-policy overlays, so a client that edited or removed `tools.policy` can send `reload` to make the live catalogue reflect the durable config rather than stale session-only policy. A forced reload observes the files it read, so the automatic poll before the next prompt or `set_model` does not rebuild again. The rebuild runs off the dispatch runtime (`tokio::task::spawn_blocking`); connections keep being accepted and read and events keep flushing while it is in flight; commands are dispatched in order once the rebuild completes, and the reply is sent once the result is applied.

**Response:** a data-less success (`success: true`) once the runtime was reloaded; `success: false` with the rebuild error (for example a config parse error) when the rebuild failed — the last-good runtime stays active — or `"provider reload is not configured"` for a loop built without a reloadable configuration.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"reload"` | yes | |
| `id` | string | no | Correlation ID |

---

### `get_subagents`

Return the current list of spawned subagents and their live status (#524).

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"get_subagents"` | yes | |
| `id` | string | no | Correlation ID |

**Response data:** an array of subagent snapshots. Each entry includes compatibility `agentId` (the user-facing display label), additive `agentUuid` (hidden durable identity for this spawn), additive `displayName` (explicit display-label alias), `status` (`starting` / `idle` / `running` / `error` / `exited`), optional `lastTool` / `lastError`, `pid`, optional `socketPath`, optional `parentId`, optional `workflow`, and `readOnly` (observer spawn with write/edit disabled). Parent tools continue to accept display labels for live subagents; clients should key durable UI/API state by `agentUuid` when present and render `displayName` / `agentId`. `status:error` and `lastError` are terminal/run-level failure signals (for example `agent_error`), not recoverable child tool `isError` results.

**Example:**

```json
{"type":"get_subagents","id":"gs-1"}
```

---

### `get_tool_catalogue` / `list_tools`

Return the rich tool catalogue snapshot for control/query clients. This is the complete bundled-native plus UDS view: each entry is a `ToolCatalogueEntry` with tool identity, description/schema, source, owner, lifecycle, configured/profile/session policy placeholders, effective availability, restriction reason, and health.

`list_tools` is accepted as a wire alias and responds with command `get_tool_catalogue`.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"get_tool_catalogue"` or `"list_tools"` | yes | |
| `id` | string | no | Correlation ID |

**Response data:**

```json
{
  "tools": [
    {
      "name": "weather",
      "description": "Get current weather for a city",
      "source": "uds",
      "owner": "uds:client:1",
      "availability": "enabled",
      "lifecycle": "runtime-loadable",
      "effectiveEnabled": true,
      "health": "healthy"
    }
  ]
}
```

Returns an empty `tools` array only when no tools are registered in the process.

---

### `set_tool_policy`

Mutate the tool-policy overlay used by subsequent model-visible tool catalogues. By default the mutation is live-session-only. Set `persist: true` to also store successful choices in the active config; immediate requests write when they apply, and queued `atNextTurnBoundary` requests write when the boundary drains. The command is backward compatible: omitting `operation` keeps legacy patch semantics.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"set_tool_policy"` | yes | |
| `id` | string | no | Correlation ID |
| `mutations` | array | yes for patch; may be empty for replace | Listed tool policy changes. Each item identifies a tool by `toolId` (stable id, preferred) or `name`, plus `scope` (`"none"`, `"parent"`, `"child"`, or `"both"`) and optional `reason`. |
| `mode` | `"immediateIfIdle"` \| `"atNextTurnBoundary"` | no | Defaults to `"immediateIfIdle"`. Timing is unchanged by `operation`: if the agent is busy, immediate requests queue for the next boundary. |
| `operation` | `"patch"` \| `"replace"` | no | Defaults to `"patch"`. Patch changes only listed tools. Replace treats `mutations` as the complete desired profile and applies `unlistedScope` to every currently registered, unlisted tool. |
| `unlistedScope` | scope string | required when `operation` is `"replace"` | Closed-world scope for registered tools not listed in `mutations`. |
| `persist` | boolean | no | Defaults to `false`. When `true`, successful reconciled choices are written to `tools.policy.entries`: immediately for applied immediate requests, or at drain time for queued `atNextTurnBoundary` requests. |

`replace` reconciliation reports public per-tool statuses for listed and unlisted current catalogue entries (`applied`, `alreadyInState`, `blockedByRestriction`, or `unknownTool`). Known entries report the resolved catalogue `name`; when the caller supplied a different identifier such as a stable `toolId`, results include `requestedIdentifier` for audit/display. Listed unknown/removed tools remain reported as `unknownTool`; stable-id-shaped identifiers are resolved only as stable ids and do not fall through to current tool names. Registered but unlisted tools are reconciled with `unlistedScope`. Restriction ceilings still prevent widening even in replace mode.

Queued reconciliation outcomes are observable through the later `tool_policy_changed` event. When the initiating command included `id`, that event includes `correlationId` with the same value so clients can correlate application-time results to the queued request.

Examples:

```json
{"type":"set_tool_policy","mutations":[{"name":"read","scope":"child"}]}
```

```json
{"type":"set_tool_policy","operation":"replace","unlistedScope":"none","mutations":[{"toolId":"tool-read","scope":"both"}]}
```

---

### `register_tools`

Register one or more tools from a connected extension client. See Extensions guide (`docs {"name":"extensions"}`) for full details.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"register_tools"` | yes | |
| `id` | string | no | Correlation ID |
| `tools` | array | yes | Array of tool registration objects |

Each tool object:

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | Tool name (must not shadow a core tool) |
| `description` | string | yes | Description shown to the LLM |
| `parametersSchema` | string | no | JSON Schema for tool parameters. Default: `{"type":"object"}` |

**Response:**

```json
{"type":"response","id":"rt-1","command":"register_tools","success":true,"data":{"registered":["weather"]}}
```

**Side effect:** Broadcasts `tool_catalogue_changed` to connected control/query clients with `changedTools`, `before`, `after`, and `reason`.

**Failure:** Returns `success: false` if any tool shadows a core tool name. No tools from the batch are registered.

**Idempotent:** Re-registering an existing tool updates its definition.

**Example:**

```json
{
  "type": "register_tools",
  "id": "rt-1",
  "tools": [
    {
      "name": "weather",
      "description": "Get current weather for a city",
      "parametersSchema": "{\"type\":\"object\",\"properties\":{\"city\":{\"type\":\"string\"}},\"required\":[\"city\"]}"
    }
  ]
}
```

---

### `unregister_tools`

Remove previously registered tools by name.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"unregister_tools"` | yes | |
| `id` | string | no | Correlation ID |
| `tools` | array | yes | Array of tool name strings to remove |

**Response:**

```json
{"type":"response","id":"ut-1","command":"unregister_tools","success":true,"data":{"unregistered":["weather"]}}
```

**Side effect:** Broadcasts `tool_catalogue_changed` to connected control/query clients with `changedTools`, `before`, `after`, and `reason`.

Unknown tool names are silently ignored (not an error).

**Example:**

```json
{"type":"unregister_tools","id":"ut-1","tools":["weather"]}
```

---

### `tool_result`

Return the result of a tool execution request. Sent by an extension client in response to an `execute_tool` event.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"tool_result"` | yes | |
| `toolCallId` | string | yes | Must match the `toolCallId` from the `execute_tool` event |
| `content` | string | yes | Result text returned to the LLM |
| `isError` | boolean | no | `true` if the result represents an error. Default: `false` |

**No response event.** The result is delivered directly to the agent loop.

**Example:**

```json
{"type":"tool_result","toolCallId":"uds-0000000abc-00000001","content":"22°C, sunny","isError":false}
```

---

### `bind_parent_control`

Launcher-only. A harness started by another harness (`--spawned
--parent-control <sidecar>`) accepts **exactly one** connection presenting
the capability its launcher minted for it; the launcher's monitor sends this
as the first frame on that connection (#1935). Ordinary clients never send
it.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"bind_parent_control"` | yes | |
| `generation` | integer | yes | The launch generation the sidecar carried |
| `capability` | string | yes | 64 lowercase hex characters, exactly the sidecar's material |

**Response:** `{"type":"response","command":"bind_parent_control","success":true}`
on the accepted connection. A missing, mismatched, malformed, replayed or
second presentation — or any presentation to a harness that was not launched
with a parent control sidecar — **closes the presenting connection** with no
response (fail closed).

The bound connection carries the `BoundParent` authority for `shutdown` and
`terminate_delegated_agent`. Its EOF or reset is the only client disconnect
that ends the harness: the common shutdown runs with reason
`parent_connection_lost`. Every other client disconnect, including the last
one, leaves a launched harness running. A launched harness that no parent
binds within its bind deadline (30 s; `QUECTO_PARENT_BIND_DEADLINE_MS`) runs
the same shutdown with reason `parent_never_bound`.

---

### `shutdown`

Ask the harness to end itself and its directly owned subtree (#1934, live
since #1935). Authorized on the bound parent connection and, until peer
authentication exists on the harness's own socket, on any local client.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"shutdown"` | yes | |
| `id` | string | no | Correlation id echoed on the response |
| `reason` | string | yes | One of `parent_shutdown`, `selected_termination`, `parent_connection_lost`, `parent_never_bound`, `termination_signal`, `operator_request` |

**Response:** `{"type":"response","id":…,"command":"shutdown","success":true,"data":{"status":"shutting_down","reason":…}}`
is written and flushed **before** any teardown effect; the harness then
cancels the in-flight turn, sends `shutdown` to each direct child, persists
the session and exits. A duplicate `shutdown` joins the admission already in
progress. Unknown fields, unknown reasons and an already terminated harness
are rejected with a correlated `success:false` response.

### `terminate_delegated_agent`

Resolve exactly one direct edge toward a selected descendant (#1934). The
target must be known, identified by uuid **and** launch generation, and
reachable within `remaining_depth` (1..=32) hops; intermediates stay alive.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"terminate_delegated_agent"` | yes | |
| `id` | string | no | Correlation id echoed on the response |
| `target_uuid` | string | yes | |
| `target_generation` | integer | yes | |
| `remaining_depth` | integer | yes | |

**Response:** `data.status` is `shutdown_requested` (the target was a direct
child and received `shutdown`) or `forwarded` (one hop consumed via
`via_uuid`, `remaining_depth` decremented). Refused routes touch no edge.

---

## Events (agent → client)

Events are emitted as length-prefixed JSON frames. Every connected client receives every event (broadcast model).

### `agent_start`

Emitted when the agent begins processing a prompt.

```json
{"type":"agent_start"}
```

### `agent_end`

Emitted when the agent finishes processing a prompt run. After #1060 / ADR-0008 part 2, full message bodies are **not** re-carried: `messages` is always empty and clients resolve content via `get_message` using the stable refs (or from stream tokens they already held).

```json
{"type":"agent_end","messages":[],"messageRefs":["8f14e45f-ceea-4670-9f5c-2f1a72f1a72f"]}
```

### `workflow_idle`

Emitted after the post-turn drain finds no further workflow continuation runnable. Optional `reason` distinguishes intervention-worthy exhaustion from deliberate stops so supervisors do not alert on an abort they requested:

| `reason` | Meaning |
|---|---|
| `exhausted` | Auto-continuation gave up (no-progress / nudge cap / unfinished with no nudge) |
| `explicit_abort` | Parent explicitly aborted |
| `completed` | Workflow reached a terminal state (or none bound) |

```json
{"type":"workflow_idle","reason":"exhausted"}
```

### `token`

Incremental text token from the LLM during streaming. Tokens arrive in real time as the model generates them.

```json
{"type":"token","token":"Hello"}
```

### `thinking`

Display-safe model thinking/reasoning text from the LLM during streaming. Thinking is additive protocol data and is never part of answer `token` text. Providers may also expose redacted/private thinking metadata internally; UDS only carries visible text deltas and recovered messages only carry display-safe thinking blocks/placeholders.

```json
{"type":"thinking","text":"I should compare the alternatives."}
```

### `turn_start`

A new LLM call begins (the agent may make multiple calls per prompt if tools are involved).

```json
{"type":"turn_start"}
```

### `turn_end`

An LLM call completed. After #1060 the assistant body is not re-carried on the wire: use stream tokens and/or `messageRefs` + `get_message`. Occupancy fields power the TUI context gauge.

```json
{
  "type": "turn_end",
  "message": {
    "role": "assistant",
    "content": "",
    "messageRefs": ["9b74e45f-ceea-4670-9f5c-2f1a72f1a730"],
    "usage": {"input": 150, "output": 42, "total": 192},
    "stopReason": "end_turn",
    "contextTokens": 4200,
    "maxContextTokens": 200000,
    "contentLength": 128
  },
  "toolResults": []
}
```

### `tool_execution_start`

A tool began executing. Includes the tool call ID (for correlation with `tool_execution_end`), tool name, and arguments.

```json
{
  "type": "tool_execution_start",
  "toolCallId": "call_abc123",
  "toolName": "bash",
  "args": {"command": "ls -la"}
}
```

### `tool_execution_end`

A tool finished executing. Includes the result and whether it was an error.

```json
{
  "type": "tool_execution_end",
  "toolCallId": "call_abc123",
  "toolName": "bash",
  "result": {"content": [{"type": "text", "text": "file1.txt\nfile2.txt"}]},
  "isError": false
}
```

### `workflow_state`

Emitted whenever an agent's workflow advances (template selected, step completed, mode change). Every `workflow_state` event is **identity-tagged** so any consumer can rebuild the unit tree from the stream alone:

```json
{
  "type": "workflow_state",
  "agent_id": "reviewer",
  "parent_id": "root",
  "mode": "active",
  "progress": {"done": 2, "total": 5}
}
```

| Field | Type | Description |
|---|---|---|
| `agent_id` | string \| null | The emitting agent's id (its session name); `null` if unnamed |
| `parent_id` | string \| null | The spawning agent's id; `null` at the root. Sourced from `--parent-id` (set automatically by `spawn`) |
| `mode` | string | Workflow mode (`selecting_template` / `active` / `complete`) |
| `progress` | object | `{done, total}` step counts (plus `percent` on the emitter's own events) |

**Forwarding (push observability).** A parent's per-child monitor re-emits each child's `workflow_state` events onto the **parent's** stream, re-stamped with the child's identity. Forwarded events are rebuilt **canonically** — only `type`, `agent_id`, `parent_id`, `mode`, and `progress` are carried; arbitrary child-supplied keys are not passed through. This lets a supervisor observe a whole subtree's progress from a single socket without polling each child (PRD Stage B).

### `response`

Direct response to a command. Carries the correlation `id` (if one was sent), the command name, success/failure, and optional data or error message.

```json
{"type":"response","id":"req-1","command":"prompt","success":true}
```

```json
{"type":"response","id":"sm-1","command":"set_model","success":false,"error":"set_model requires model, or provider+modelId"}
```

```json
{"type":"response","command":"agent_error","success":false,"error":"no configured provider matches model prefix 'gemini'"}
```

> **Note:** Agent errors from LLM failures are emitted as `response` events with `command: "agent_error"` rather than `command: "prompt"`. This distinguishes infrastructure errors from successful completions.

### `execute_tool`

Sent to the specific extension client that registered a tool when the LLM calls it. This event is **routed**, not broadcast — only the registering client receives it.

```json
{
  "type": "execute_tool",
  "toolCallId": "uds-0000000abc-00000001",
  "toolName": "weather",
  "arguments": "{\"city\":\"London\"}"
}
```

| Field | Type | Description |
|---|---|---|
| `toolCallId` | string | Unique call identifier — must be echoed back in `tool_result` |
| `toolName` | string | Name of the tool being called |
| `arguments` | string | JSON string of the tool arguments from the LLM |

The extension must respond with a `tool_result` command containing the matching `toolCallId`. If no response arrives within 30 seconds, the agent returns a timeout error to the LLM.

### `tool_catalogue_changed`

Broadcast when the rich tool catalogue changes after `register_tools`, `unregister_tools`, or client disconnect. Contains changed tool names, the previous catalogue snapshot, the new snapshot, and a reason.

```json
{
  "type": "tool_catalogue_changed",
  "changedTools": ["weather"],
  "before": [],
  "after": [
    {"name": "weather", "description": "Get current weather for a city", "source": "uds", "owner": "uds:client:1"}
  ],
  "reason": "register_tool"
}
```

### `subagent_notification`

Passive child-agent notification for human/UI visibility (completion, error, exit).

```json
{"type":"subagent_notification","agentId":"reviewer","sequence":3,"message":"child exited"}
```

### `subagent_state_changed`

Broadcast replacement snapshot of all spawned subagent statuses (clients do a simple replace). Entries match the `get_subagents` shape, including `readOnly`.

```json
{"type":"subagent_state_changed","subagents":[{"agentId":"reviewer","agentUuid":"f47ac10b-58cc-4372-a567-0e02b2c3d479","displayName":"reviewer","status":"idle","pid":1234,"readOnly":true}]}
```

### `admission_state_changed`

Broadcast after every inference-admission transition of this process (#1679
P4): an attempt queued, granted, completed, refused, cancelled or abandoned,
or a group's cooldown learned. Carries the full bounded `admission` object
described under [`get_state`](#get_state). Events are delivered in
`revision` order, so clients do a simple replace. Emitted only when the
process joined an admission authority; the same transition also advances the
`get_state` generation.

**Forwarding (push observability).** A parent's per-child monitor re-emits a
child's `admission_state_changed` onto the parent's stream, re-stamped with
`agent_id` / `parent_id` (a forwarded grandchild keeps its own identity) and
rebuilt from the known fields above only (groups bounded to 32;
`authorityStatus` only from its closed vocabulary, with `connected`/`epoch`;
never `directory`), so a supervisor sees a descendant waiting for admission —
or one whose capability is gone (`authorityStatus: "unavailable"` after a
reset, to be respawned) — without polling each child socket. An identity embedded by the child is honoured only for a registered
descendant of that child (never the parent or a sibling); anything else is
stamped as the child itself. Events without `agent_id` are the connected
agent's own.

```json
{"type":"admission_state_changed","admission":{"waiting":1,"admitted":0,"longestWaitSeconds":3,"groups":[{"group":"anthropic"}],"counters":{"completed":0,"refused":0,"cancelled":0,"abandoned":0},"hidden":0,"revision":1}}
```

### `subagent_messages_appended`

Emitted when a (sub)agent completes a turn, carrying stable refs for messages appended during that turn. A child emits this on its own stream; the parent's monitor may re-stamp `agent_id` and forward it so inspectors can stream child output turn-by-turn without re-carrying full bodies (#1060).

```json
{"type":"subagent_messages_appended","agent_id":"reviewer","messages":[],"messageRefs":["…"]}
```

### `error` (lagged client)

Sent when a client falls behind on the broadcast channel (buffer overflow). The client should call `get_messages` to re-sync.

```json
{"type":"error","message":"dropped 12 events — use get_messages to re-sync"}
```

---

## Error handling

- **Malformed JSON:** Returns `response` with `command: "parse_error"` and `success: false`. The `error` text preserves the detailed serde parse error in both single-client and multi-client modes; clients that previously string-matched the old generic `"invalid JSON command"` text should switch to the structured `command: "parse_error"` / `success: false` fields.
- **Unknown command type:** Returns `response` with `success: false`
- **Line/frame too long:** Oversized inbound messages are rejected against the shared **8 MiB** protocol cap (`quecto-line-io`); clients should recover large content via ranged `get_message` rather than a single oversized frame
- **Agent error during prompt:** Returns `response` with `command: "agent_error"`. The agent stays alive — subsequent commands are processed normally
- **Unroutable model:** If `set_model` was set to a provider that doesn't exist, the next `prompt` returns an `agent_error` with `"no configured provider matches model prefix 'X'"`. Use `set_model` to switch to a valid model and retry

---

## Event sequence diagrams

### Simple prompt (no tools)

```
Client                          Agent
  │                               │
  │──prompt──────────────────────>│
  │                               │
  │<──────────────agent_start─────│
  │<──────────────turn_start──────│
  │<──────────────token───────────│  (repeated)
  │<──────────────turn_end────────│
  │<──────────────agent_end───────│
  │<──────────────response────────│  command:"prompt", success:true
```

### Prompt with tool call

```
Client                          Agent
  │                               │
  │──prompt──────────────────────>│
  │                               │
  │<──────────────agent_start─────│
  │<──────────────turn_start──────│
  │<──────────────token───────────│
  │<────────tool_execution_start──│  toolName:"bash"
  │<────────tool_execution_end────│  result, isError
  │<──────────────turn_end────────│  (LLM processes tool result)
  │<──────────────agent_end───────│
  │<──────────────response────────│  command:"prompt", success:true
```

### Prompt with follow-up

```
Client                          Agent
  │                               │
  │──follow_up───────────────────>│
  │<──────────────response────────│  command:"follow_up", success:true
  │                               │
  │──prompt──────────────────────>│
  │                               │
  │<──────────────agent_start─────│  (prompt run)
  │<──────────────...─────────────│
  │<──────────────agent_end───────│
  │<──────────────response────────│  command:"prompt", success:true
  │                               │
  │<──────────────agent_start─────│  (follow-up run — automatic)
  │<──────────────...─────────────│
  │<──────────────agent_end───────│
```

### Abort during prompt

```
Client                          Agent
  │                               │
  │──prompt──────────────────────>│
  │<──────────────agent_start─────│
  │<──────────────turn_start──────│
  │                               │
  │──abort───────────────────────>│
  │<──────────────response────────│  command:"abort", success:true
  │                               │  (agent cancelled, no agent_end)
```

### Error and recovery

```
Client                          Agent
  │                               │
  │──set_model("gemini/pro")─────>│
  │<──────────────response────────│  command:"set_model", success:true
  │                               │
  │──prompt──────────────────────>│
  │<──────────────agent_start─────│
  │<──────────────turn_start──────│
  │<──────────────response────────│  command:"agent_error", error:"no configured provider..."
  │                               │
  │──set_model("anthropic/...")──>│
  │<──────────────response────────│  command:"set_model", success:true
  │                               │
  │──prompt──────────────────────>│  (succeeds now)
  │<──────────────agent_start─────│
  │<──────────────...─────────────│
  │<──────────────agent_end───────│
  │<──────────────response────────│  command:"prompt", success:true
```

### Extension registration and tool execution

```
Extension Client                Agent                    Other Clients
  │                               │                          │
  │──register_tools──────────────>│                          │
  │<──────────────response────────│  success:true            │
  │                               │──tool_catalogue_changed─────>│
  │                               │                          │
  │              ... LLM calls the registered tool ...       │
  │                               │                          │
  │<──────────execute_tool────────│  (routed, not broadcast) │
  │                               │                          │
  │──tool_result─────────────────>│                          │
  │                               │                          │
  │  ... tool_execution_start/end broadcast to all ...       │
  │<────tool_execution_end────────│──tool_execution_end─────>│
```

### Extension disconnect cleanup

```
Extension Client                Agent                    Other Clients
  │                               │                          │
  │──[disconnect]────────────────>│                          │
  │                               │  (auto-unregister tools) │
  │                               │──tool_catalogue_changed─────>│
```

---

## Connecting with common tools

### socat

```bash
socat - UNIX-CONNECT:/tmp/quecto-agent-<uuid>.sock
```

### Python

```python
import socket, json

sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.connect("/tmp/quecto-agent-<uuid>.sock")

def send(cmd):
    sock.sendall((json.dumps(cmd) + "\n").encode())

def recv_lines():
    buf = b""
    while True:
        data = sock.recv(4096)
        if not data:
            break
        buf += data
        while b"\n" in buf:
            line, buf = buf.split(b"\n", 1)
            yield json.loads(line)

send({"type": "prompt", "id": "p1", "message": "Hello!"})
for event in recv_lines():
    print(event["type"], event.get("command", ""))
    if event.get("type") == "response" and event.get("command") == "prompt":
        break

sock.close()
```

### Node.js

```javascript
const net = require('net');
const readline = require('readline');

const sock = net.createConnection('/tmp/quecto-agent-<uuid>.sock');
const rl = readline.createInterface({ input: sock });

rl.on('line', (line) => {
  const event = JSON.parse(line);
  console.log(event.type, event.command || '');
});

sock.write(JSON.stringify({type: 'prompt', id: 'p1', message: 'Hello!'}) + '\n');
```

---

## Startup flags reference

All flags for `quecto agent` that affect UDS mode:

| Flag | Description |
|------|-------------|
| `--mode uds` | Required. Run in UDS mode |
| `--socket <path>` | Explicit socket path (max 104 bytes). Default: auto in `$XDG_RUNTIME_DIR` or `$TMPDIR` |
| `-s` / `--session <name>` | Named session for persistence. Default: `cli:default` |
| `--no-session` | Ephemeral mode — no session saved/loaded |
| `--system <text>` | System prompt (not persisted in session) |
| `--model <model>` | Override default model from config |
| `--max-iterations <n>` | Max tool call rounds per prompt |
| `--max-time <secs>` | Wall-clock timeout for the entire agent |
| `--persist` | Keep agent alive after all clients disconnect (top-level only; refused with `--parent-control`) |
| `--effort <level>` | Reasoning effort (`none`/`low`/`medium`/`high`/`xhigh`/`max`). Provider vocabulary still applies at request time. Overrides config and env var |
| `--workflow` | Start workflow-driven prompt injection immediately |
| `--workflow-guards` | Enable workflow bash command guards |
| `--no-workflow` | Disable workflow tool/state/prompt |
| `--parent-id <id>` | Declare this agent's parent in the unit tree (set automatically by `spawn`) |
| `--disable-tool <name>` | Disable/hide a tool and deny re-registration (repeatable) |
| `--config <path>` | Override config file path |

> **Note:** `bash` commands run natively in the workspace and can reach
> `$HOME` (e.g. `gh` credentials, `.gitconfig`). To confine command
> execution, run Quecto inside a container.

## See also

- Extensions (`docs {"name":"extensions"}`) — adding custom tools via native config or UDS registration
- Subagents (`docs {"name":"subagents"}`) — spawning child agent processes from within a session
- Workflow Automation (`docs {"name":"workflow"}`) — configurable step-by-step development process

## Swarm supervision additions

`{"type":"swarm_control","id":"pause-1","action":"pause","reason":"approval"}`
applies a durable pause independently of the prompt queue. Actions are `pause`,
`resume`, `close`, `extend`, `status`, and `usage_budget`; `usage_budget` requires
`token_limit` (positive integer or explicit null) and optionally `strict_unknown`
(default true); `extend` requires `deadline_seconds` (positive integer). Add
`agent_id` to route to a descendant. Successful responses include `applied`,
`status`, `generation`, `budget`, and, for a run holding a proposed outcome
(#1729), `outcome` and `reason`. Every end of a run is such a pause; only these
supervisor controls resume (`resume`) or finish (`close`) it. The internal `wake` action carries a durable
event generation and is rechecked for actionability at dispatch; it answers
`{"status":"accepted"}` when a wake turn is queued, `{"status":"coalesced"}` when
the generation joined an already pending wake, or an error (`wake unavailable`,
`wake queue full`) when nothing would drain it — the durable inbox remains
authoritative in every case.

`get_state` adds bounded `controlReceipts` identified by command ID, with queued,
started, completed, failed, cancelled or rejected status. Completed means a model
turn completed, not that its instructions were semantically fulfilled.

`{"type":"get_report","id":"report-1","agent_id":"...","export_raw":true}`
returns the latest assistant report and optional retained raw export paths plus
checksum. Omit `agent_id` for the connected agent and `export_raw` for report-only
inspection. It is available while busy, does not advance unread history cursors,
and includes `get_message` recovery for truncated content. Artifact paths are
local to the responding runtime. See [swarm diagnostics](swarm.md#operational-diagnostics-and-budgets)
for retention, accounting availability, provenance and export consistency.
