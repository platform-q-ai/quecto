# Subagents

Subagents are child quecto processes spawned by the `spawn` tool during an
agent session. They run as **background UDS-mode agents** — the parent returns
immediately and interacts with children asynchronously via the `agent_cmd`
tool. No external dependencies (no `ncat`, `socat`, or `bash` intermediary).

## Parent coordination model

Use subagents to isolate substantial working context and run independent work
in the background while the parent remains available to the user. Delegate
deliberately — not as the default for every non-trivial request. Decide by the
**shape and context cost** of the work, not merely whether it sounds complex.

### Handle directly in the parent

Keep work in the parent when it is focused, short-lived, and low-context,
especially when:

- the relevant file, symbol, command, or value is already known;
- a single-fact lookup or answer should require only a few targeted tool calls;
- the expected tool output is small and directly useful to the final answer;
- the task is a small, bounded edit or verification;
- the work is clarification, synthesis, final judgment, or user-facing
  coordination;
- delegating, briefing a child, and retrieving its result would cost more than
  executing directly.

If the scope is uncertain, begin with a focused parent search. Delegate only
when that probe shows the work is broader, longer, noisier, or more
context-heavy than expected.

### Delegate to a subagent

Delegate when one or more of these applies:

- answering requires a broad or uncertain search across several files,
  directories, subsystems, or naming conventions;
- the work will produce substantial file excerpts, command output, or
  intermediate evidence while the parent needs only the conclusion and concise
  supporting evidence;
- the task is long-running or likely to require many tool calls;
- the work is independently separable and can run in parallel with other
  useful work;
- a specialized research, implementation, debugging, or review perspective
  would materially help;
- an available workflow provides useful sequencing, verification, evidence
  gates, or review structure.

The number of files alone is not an absolute rule. Read several small, known
files directly when that is cheaper; delegate when the search is broad,
uncertain, noisy, or likely to consume substantial parent context.

Once a scope is delegated, **do not repeat the same investigation in the
parent**. Continue distinct coordination, synthesis, user interaction, or
independent work. Checking a critical citation or running a focused command to
verify a child's conclusion is not duplication; repeating the child's full
search is.

### Delegation ownership

Give each child one clear goal, ownership boundary, and expected deliverable.
Do not create redundant children for the same question. Parallelize only across
distinct workstreams or review dimensions.

A child should absorb the detailed working context and return a **concise
report** containing conclusions, material evidence, uncertainty, and relevant
`file:line` citations. Do not ask it to return raw file dumps unless those are
the requested deliverable.

The parent retains responsibility for:

- the user conversation and clarification of intent;
- coordination across workstreams;
- checking that a child's report answers its assigned scope;
- verifying important or surprising claims where appropriate;
- deduplicating and reconciling conflicting child results;
- making the final judgment;
- synthesizing and relaying what matters to the user.

A child's report is **input to the parent's answer**, not a substitute for the
parent's judgment.

### Choosing how to delegate

| Approach | When to use |
|----------|-------------|
| Plain child `task` | Substantial but focused or exploratory work that does not need a prescribed multi-step process |
| `workflow: true` | Work is workflow-shaped and the child should inspect templates in its config and select the best match |
| Instruct child to `select_template` | A specific existing template is clearly appropriate |
| `workflow_spec` | Child must follow an exact, observable, auditable sequence (known template or a new one). Bind the full template rather than relying on prose |
| `read_only: true` | Reviewers, researchers, and other non-editing children |

For coding tasks in this repo, prefer delegating to children that run the
repository workflows (`feature`, `bugfix`, `refactor`, `remove`, `chore`,
`adversarial-review`, `investigate`, `flake-hunt`, `plan`, `prd`). See the
`workflow` doc (`docs {"name":"workflow"}`) for template selection and step
progression.

### Briefing children

Children have **separate LLM contexts** and do not automatically inherit the
parent's conversation. Give each child the context required to work
independently.

Give relevant children the same engineering constraints as the parent: prefer
minimal, purpose-aligned changes; follow repository conventions; apply YAGNI,
BDD/TDD, and Clean Architecture principles where practical; run appropriate
verification; and never bypass hooks with `--no-verify`.

Children should execute their assigned work directly unless instructed
otherwise by an attached workflow.

### Reusing child context

Reuse a child that already owns the relevant context instead of starting
redundant work:

- use `prompt` to give a live idle child related work;
- use `follow_up` to queue related work after its current run;
- use `steer` to interrupt and redirect active work;
- spawn a new child for a new independent scope;
- retrieve needed results with `get_messages` before cleanup.

Do not reuse stale child context merely to avoid a new session; use it only
when its prior context is relevant and safe for the new assignment.

### Non-blocking execution and result recovery

`spawn` returns when the child **socket** is ready — not when the task finishes.
Completion is **multi-turn**. The `agent_cmd await` command has been removed; use the passive completion note plus `get_messages` path below.

#### Required sequence

1. **Spawn** (and brief the child). Save the returned UUID for every child-targeted `agent_cmd` call; the spawn `agent_id` is a UI label only.
2. **End this parent turn** (or do other *non-duplicative* work that does not need
   the child’s answer). Stay available to the user.
3. **Next turn:** a passive one-line completion note arrives automatically when
   the child finishes/errors/exits.
4. **Then** plain `agent_cmd get_messages` (omit/null `count` and `before`) for the default unread report.
5. Verify, synthesize, and answer the user. Relay conclusions — not raw child dumps
   unless asked.

#### Do not

- Poll `get_subagents`, `get_subagents_all`, or `get_state` in a loop waiting for idle.
- `sleep` / bash-wait / busy-wait for the child in the same turn.
- Treat the passive note as the child’s report — always `get_messages` for content.

#### Optional tools (not wait loops)

- `get_state` — occasional live progress/debug.
- `get_subagents_all` with `agent_id: "*"` — parent/session-wide inventory and cleanup **after** coordination, not completion waiting.
- `get_subagents` — send to a specific live subagent to list only that agent's nested children; a child with no nested agents returns `subagents: []`.
- `abort` / `kill` — stop work or the process when needed.

If you need the child’s answer before you can help the user, **yield the turn**
and continue when the note arrives; do not invent a same-turn wait.


### Safety for delegated work

- Children inherit the parent's credentials and tool policy. Do not
  broaden a child's practical authority beyond the user's intent.
- `read_only: true` disables and hides the `write` and `edit` tools from the
  child's model-visible tool definitions but is **not a hard sandbox** because
  the child retains `bash`. Explicitly prohibit mutation and verify the
  workspace diff after read-only children finish before trusting that they made
  no changes.
- Never print secrets. Have children use configured local tools without echoing
  credentials.
- Avoid redundant agents, but use parallelism across genuinely distinct
  workstreams when it provides value.

## Overview

When the LLM calls the `spawn` tool, quecto launches a new `quecto agent`
process in UDS mode (`--mode uds --spawned --parent-control <sidecar>`; never
`--persist`, see [Lifetime](#lifetime-and-session-restore-1937)). The child process:

- Uses the same quecto binary (`std::env::current_exe()`)
- Inherits the parent's `QUECTO_BASE_DIR` (config, credentials, sessions)
- Gets its own UUID minted per spawn and returned to the parent; use that UUID as `agent_cmd.agent_id`, not the spawn display label
- Listens on a Unix domain socket for commands
- Runs in the background — the parent is **not blocked**

The parent interacts with the child using the `agent_cmd` tool, which
connects to the child's UDS socket directly from Rust.

## Tools

### `spawn` — launch a subagent

```json
{
  "type": "object",
  "properties": {
    "task": {
      "type": "string",
      "description": "Initial task to send (optional — starts idle if omitted)"
    },
    "agent_id": {
      "type": "string",
      "description": "Optional UI display label; use the UUID returned by spawn for agent_cmd"
    },
    "system": {
      "type": "string",
      "description": "System prompt for the subagent"
    },
    "config": {
      "type": "string",
      "description": "Path to a config file to pass to the child via --config (optional)"
    },
    "model": {
      "type": "string",
      "description": "Model for the child in provider/model form (e.g. 'openai-api/gpt-5.5'), same format as agent_cmd set_model. Forwarded as --model at launch so the child's first turn runs on it"
    },
    "provider": {
      "type": "string",
      "description": "Provider name for the child model (alternative to model; must be paired with model_id)"
    },
    "model_id": {
      "type": "string",
      "description": "Model id for the child model (used with provider)"
    },
    "effort": {
      "type": "string",
      "description": "Reasoning effort for the child (one of: none, low, medium, high, xhigh, max). Forwarded as --effort at launch. Validated against the target model's vocabulary when a model is given."
    },
    "workflow": {
      "type": "boolean",
      "description": "Start the child with --workflow (model selects a template itself)"
    },
    "workflow_spec": {
      "type": "object",
      "description": "Assign a specific workflow to the child by value: { \"template\": { full inline template } }. The child runs exactly that template, bound, in Active mode — no model-driven selection."
    }
  }
}
```

- **`task` is optional.** Omitting it creates an idle agent ready for prompts via `agent_cmd`.
- **`agent_id`** on `spawn` is a UI display label and must be unique among live subagents. Save the returned UUID and use it for all child-targeted `agent_cmd` calls.
- Returns immediately (< 1 second) after the child's socket is ready.
- **`workflow_spec` vs `workflow`.** `workflow: true` makes the workflow tool available so the *child* picks a template; `workflow_spec` hands the child a specific template **by value** and binds it. They are independent of `config`, which supplies the child's runtime (providers/model/default template library).
- **`model` (optional).** Sets the child's model at launch — accepts either a full `provider/model` string (e.g. `openai-api/gpt-5.5`) or a `provider` + `model_id` pair, the same format(s) as `agent_cmd set_model` (and validated by the same logic). It is forwarded to the child as `--model`, so the child's **first turn** (if `task` is given) already runs on the chosen model — no follow-up `set_model` round-trip needed. **Precedence:** an explicit `model` arg wins over any model from a forwarded `--config`, which wins over the built-in default. An invalid combination (e.g. `provider` without `model_id`) is a clear spawn error rather than a silent fall-back to the default.
- **`effort` (optional).** Sets the child's reasoning effort at launch. Must be one of `none`, `low`, `medium`, `high`, `xhigh`, `max`; when a `model` is also given, the value is additionally checked against that model's effort vocabulary (e.g. OpenAI reasoning models take `none`–`xhigh`; Anthropic 4.6 models take `low`/`medium`/`high`/`max`). Invalid or non-string values are rejected at spawn parse time with an error listing the valid levels. It is forwarded to the child as `--effort`, so the child's **first turn** already runs at the chosen effort. It can be changed on a running child with `agent_cmd set_effort` (or from the TUI effort selector while that child is focused). **Precedence:** explicit spawn `effort` > the child's forwarded `agents.defaults.effort` (from `--config`) > inherited `QUECTO_AGENTS_DEFAULTS_EFFORT` env > the provider default. A running child's effort is reset to the child's own default when its session is reset/resumed (`reset_effort_to_default`).

#### Spawning read-only (`read_only` / `disable_tools`)

A parent can launch a child with specific tools disabled, so the child cannot use
them through the model-visible tool surface:

- **`disable_tools` (optional).** An array of tool names (e.g. `["write",
  "edit"]`). Each named tool is **disabled before the child session starts**:
  it remains registered/described for policy/UI callers, but is hidden from the
  child's model-visible tool definitions, rejects execution, and is denied from
  later UDS/runtime re-registration — defense-in-depth beyond a prompt instruction.
- **`read_only` (optional).** A convenience that expands to
  `disable_tools: ["write", "edit"]`. The child keeps `bash`, `read`, `grep`,
  `find` and `agent_cmd` model-visible, while the `"write"` and `"edit"` tools
  are hidden and disabled.

This is the recommended posture for reviewers, which should inspect and report
but not mutate the repo:

```json
{
  "name": "spawn",
  "arguments": {
    "agent_id": "pr-reviewer",
    "task": "Review PR #123 for security issues and post inline findings",
    "read_only": true
  }
}
```

**Caveat — this is not a hard sandbox.** Disabling `write`/`edit` stops those
tools from appearing in model-visible definitions and from executing, but a child can still mutate via `bash` (e.g. `sed`, `>` redirects). Reviewers keep `bash`/`read`/`grep`/`find`/`agent_cmd` precisely so
they can fetch a diff and post comments; treat `read_only` as a guard against
accidental writes, not an isolation boundary. For stronger guarantees use container/OS isolation. For a top-level agent,
the CLI equivalent for hiding tools is `--disable-tool` (repeatable; see the README).

**Example:**

```json
{
  "name": "spawn",
  "arguments": {
    "task": "Review all Python files in src/ for security vulnerabilities",
    "agent_id": "security-reviewer",
    "system": "You are a security expert."
  }
}
```

**Return value:**

```
Subagent 'security-reviewer' is running. Use agent_cmd to interact.
```

#### Assigning a bound workflow (`workflow_spec`)

A parent can hand a child a specific workflow **by value** — the full template
travels in the spawn call, so the child does not need that template in its own
config. The assigned child starts in **Active** mode bound to exactly that
template: it cannot select a different template, and on completion it reports
its result rather than picking a new workflow.

**When to bind a workflow.** Reach for `workflow_spec` when the child must follow
an exact, multi-step process you control — e.g. a PR review that must analyze →
test → report, or a fix that must go RED → GREEN → review → merge. Use a plain
`task` for open-ended work, and `workflow: true` when you want the *child* to
pick its own template from its config. Binding makes the child's steps
observable and gates its completion on actually finishing them.

```json
{
  "name": "spawn",
  "arguments": {
    "agent_id": "pr-reviewer",
    "task": "Review PR #682",
    "workflow_spec": {
      "template": {
        "id": "review-pr",
        "label": "Review PR",
        "description": "Analyze a diff and run the test suite",
        "steps": [
          { "key": "analyze", "label": "Analyze the diff", "phase": "review" },
          { "key": "verify",  "label": "Run the test suite", "phase": "review" }
        ]
      }
    }
  }
}
```

- The `template` is a fully resolved, inlined `WorkflowTemplate`: unlike a
  canonical `workflows/*.json` directory file, it **requires** an `id` and its
  `steps` cannot use file references. Each step has `key`, `label`, `phase`, and
  optional `guidance`; see the `workflow` doc (`docs {"name":"workflow"}`) for
  the full field reference.
- The spec is size-bounded (256 KiB) and written to a private, single-use file
  the child deletes once read.
- If a spec is assigned but cannot be loaded, the child **fails closed** (it
  refuses to start a workflow rather than falling back to free selection).
- `workflow_spec` cannot be combined with the child's `--no-workflow`.

**Monitoring a child's workflow.** A workflowed child (bound or self-selected)
reports progress without polling:

- its `workflow_state` events are forwarded onto your event stream as steps
  advance (tagged with the child's `agent_id`, so a grandchild's progress is
  attributed to the grandchild, not the child);
- `agent_cmd get_state` is the live in-flight supervision API. It returns the
  child's slim state/effort/model/progress projection, `generation`, and (only
  when selected) slim workflow identity plus current step on demand — including
  while the child is mid-turn. Pass `since` to receive only
  `{"unchanged": true, "generation": N}` when nothing changed;
- in the TUI, the selected child renders its own workflow status bar.

Read the child's final result the usual way — its one-line auto-note at your next
idle turn, then `agent_cmd get_messages` for the bounded report (see [Notification model](#notification-model)).

### `agent_cmd` — interact with a subagent

Always target the UUID returned by `spawn`, never its UI display label (except `"*"` for inventory/container commands).

First bare `get_messages` (omit/null `count` and `before`) returns the latest substantive assistant message, not the entire transcript. Subsequent bare calls return unread deltas across roles; when nothing new is available, `data` is `{ "unchanged": true }`. The report cursor advances only when the result is successfully delivered to the parent model, not merely fetched. Explicit non-null `count` and/or `before` requests are cursor-neutral history pages; `before` pages backward. Reports are bounded; a busy snapshot can lag the active turn.

```json
{
  "type": "object",
  "properties": {
    "agent_id": {
      "type": "string",
      "description": "UUID returned by spawn; use * for inventory/container commands"
    },
    "command": {
      "type": "string",
      "enum": ["prompt", "steer", "follow_up", "abort", "kill",
               "get_state", "get_messages", "get_message", "get_report", "swarm_control",
               "get_session_stats", "get_subagents", "get_subagents_all",
               "get_containers", "kill_container",
               "set_model", "set_effort", "clear_history"],
      "description": "Command to send. For completed spawned work, use get_messages without count/before."
    },
    "message": {
      "type": "string",
      "description": "Message for prompt/steer/follow_up commands"
    },
    "export_raw": {"type": "boolean", "description": "With get_report, export retained wire records and spills"},
    "action": {"type": "string", "enum": ["pause", "resume", "status", "usage_budget"]},
    "reason": {"type": "string"},
    "token_limit": {"type": ["integer", "null"], "minimum": 1},
    "strict_unknown": {"type": "boolean"},
    "messageId": {"type": "string", "description": "Stable ID for get_message recovery"},
    "toolCallId": {"type": "string"},
    "offset": {"type": "integer", "minimum": 0},
    "limit": {"type": "integer", "minimum": 0},
    "count": {
      "type": "integer",
      "description": "Explicit history page size for get_messages; omit/null for the default unread report; does not move the report cursor"
    },
    "before": {
      "type": "string",
      "description": "Paging cursor for get_messages: a message id from a prior response's before field; returns the adjacent older page"
    },
    "since": {
      "type": "integer",
      "description": "Generation cursor for get_state/get_subagents; unchanged state returns only metadata"
    },
    "model": {
      "type": "string",
      "description": "set_model as provider/model; alternative to provider+model_id"
    },
    "provider": {
      "type": "string",
      "description": "set_model provider; use with model_id"
    },
    "model_id": {
      "type": "string",
      "description": "set_model id; use with provider"
    },
    "effort": {
      "type": "string",
      "enum": ["none", "low", "medium", "high", "xhigh", "max"],
      "description": "set_effort value"
    },
    "ref": {
      "type": "string",
      "description": "Container ref for kill_container, e.g. C1"
    },
    "name": {
      "type": "string",
      "description": "Container name for kill_container; alternative to ref"
    }
  },
  "required": ["agent_id", "command"]
}
```

**Supported commands:**

| Command | Description | Requires `message` |
|---------|-------------|--------------------|
| `swarm_control` | Durable pause/resume/status/usage_budget control, independent of the model queue; supports descendant routing | No |
| `get_report` | Latest substantive assistant report without advancing unread cursors; optional export_raw writes retained records and a checksum manifest | No |
| `get_message` | Recover content by stable messageId, with optional toolCallId, byte offset and limit | No |
| `prompt` | Send a task/message to the subagent | Yes |
| `steer` | Interrupt and redirect the agent (takes precedence over the workflow auto-continue nudge) | Yes |
| `follow_up` | Queue a message for after the current run | Yes |
| `abort` | Full stop: cancel the current run, kill in-flight tool/child processes, and suppress workflow auto-continue (does not resume) | No |
| `kill` | Terminate the subagent process (SIGTERM) | No |
| `get_state` | Inspect live/in-flight supervision state: slim state/effort/model/progress, generation cursor, and selected workflow identity/current step. Pass `since` for an unchanged marker | No |
| `get_messages` | Default report mode: omit/null `count` and `before`; first call returns the latest substantive assistant message, subsequent calls return unread deltas, or `unchanged` if none. The cursor advances on successful delivery to the parent model. Explicit `count` and/or `before` requests cursor-neutral history pages; `before` pages older history. A busy snapshot can lag the active turn | No |
| `get_session_stats` | Get token usage and cost | No |
| `get_subagents` | List nested subagents spawned by the targeted live subagent; not parent/session-wide inventory | No |
| `get_subagents_all` | With `agent_id: "*"`, list parent/session-wide subagent inventory for cleanup/inspection | No |
| `get_containers` | With `agent_id: "*"`, list spawned container environments | No |
| `kill_container` | With `agent_id: "*"`, terminate a spawned container by `ref` or `name` | No |
| `set_model` | Change the LLM model | No |
| `set_effort` | Change the reasoning effort (`none`/`low`/`medium`/`high`/`xhigh`/`max`, validated against the child's active model; invalid values are rejected with the valid list) | No |
| `clear_history` | Clear conversation history | No |

Tool catalogue commands (`get_tool_catalogue` / `list_tools`) remain available only on the UDS/control-plane protocol; they are not model-facing `agent_cmd` commands.

**Examples:**

```json
{"name": "agent_cmd", "arguments": {"agent_id": "<uuid-returned-by-spawn>", "command": "get_state"}}
```

```json
{"name": "agent_cmd", "arguments": {"agent_id": "<uuid-returned-by-spawn>", "command": "get_messages"}}
```

```json
{"name": "agent_cmd", "arguments": {"agent_id": "<uuid-returned-by-spawn>", "command": "steer", "message": "Focus on auth vulnerabilities only"}}
```

```json
{"name": "agent_cmd", "arguments": {"agent_id": "<uuid-returned-by-spawn>", "command": "set_effort", "effort": "high"}}
```

## Notification model

Spawned agents are completed via **non-blocking passive auto-notes**. When a
child reaches a terminal state (completed / errored / exited), the parent
automatically receives a single **one-line completion note** — no wait command is
available or required. The note is:

- **Non-blocking** — it never interrupts a running turn and never makes an idle
  parent act. It is delivered as a `role:"system"` (operator-channel) message
  and surfaces **only at the parent's next idle/turn boundary**: a completion
  that arrives while the parent is mid-turn is buffered and delivered after that
  turn finishes, never injected into an in-flight turn.
- A **one-line note**, naming the child and its outcome — it does **not** repeat
  the child's output, e.g.
  `Agent 'worker' completed and is ready for inspection`,
  `Agent 'linter' failed: …`, or
  `Agent 'worker' exited unexpectedly`. A failed note means terminal/run-level
  failure (such as `agent_error`), not every recoverable child tool `isError`.
- **Coalesced + deduplicated** — multiple completions from the same child
  collapse to one note (latest wins), so a noisy child costs at most one extra
  turn.

The note is a **summary only**; to read the child's default unread report call
plain `get_messages` (omit/null `count` and `before`). Explicit `count`/`before`
requests are cursor-neutral history pages. Intermediate
child tool errors are the child's problem: they remain visible in the child
transcript/tool stream but do not interrupt the parent, set
`get_subagents.lastError`, or mark the child `status:error` unless the run later
emits a true terminal failure signal.

### What you can see before completion

- **Workflow state changes** are forwarded onto the parent's event stream
  (identity-tagged with `agent_id` + `parent_id`). See "Observing the unit
  tree" below.
- **Live supervision** via `get_state` reports the child's current phase,
  tool activity, evidence-based recent progress, and canonical message count.
- **Stable transcript inspection** via `get_messages` is intended for committed
  or end-of-turn output. A busy response is a snapshot and can lag mutable
  in-flight transcript content.

Neither command registers as a notification. A single call that catches the
agent mid-run tells you nothing about what happens next. Do not poll these
commands in a loop waiting for completion.

### Canonical pattern: passive note + default unread report

```json
// 1. Spawn — returns when the child socket is ready, not when work is done.
{"name": "spawn", "arguments": {"agent_id": "worker", "task": "do the thing"}}

// Save the UUID returned by spawn; the placeholder below means that exact UUID.
// 2. End the parent turn or do independent work. At the next idle turn you receive:
//    Agent 'worker' completed and is ready for inspection

// 3. Inspect the unread report when the note tells you the child is done.
{"name": "agent_cmd", "arguments": {"agent_id": "<uuid-returned-by-spawn>", "command": "get_messages"}}
```

**Common mistake:** treating the one-line note as the child's result. Always
retrieve the unread report to see what the agent actually produced.

## Observing the unit tree

Every agent in a spawn hierarchy is the same quecto unit, and its
`workflow_state` events are **identity-tagged** with `agent_id` (the agent's
own id) and `parent_id` (its spawner; `null` at the root). `spawn` passes its
own id to each child automatically, so a child's events are correctly parented
without any manual flag. From these two fields alone a consumer can rebuild the
whole parent → child tree from the event stream — no per-child polling required.

Two complementary ways to observe a child's progress:

- **Pull — `get_subagents`:** each entry now carries `parent_id`, `readOnly`
  / `read_only` observer status (true when the child was spawned read-only, or
  with both `write` and `edit` disabled), and an optional `workflow` snapshot
  (`{mode, steps_completed, steps_total}`) for that child, kept current by the
  parent's per-child monitor. Good for a point-in-time view of every child at
  once.
- **Push — forwarded events:** the parent's monitor re-emits each child's
  `workflow_state` events onto the **parent's** own event stream, re-stamped
  with the child's identity. A supervisor watching one socket sees its whole
  subtree advance live. Forwarded events are rebuilt canonically (only
  `type`/`agent_id`/`parent_id`/`mode`/`progress`), so a child cannot inject
  arbitrary fields onto the parent's stream.

Prefer reading forwarded `workflow_state` events (or one `get_subagents` call)
over repeatedly polling each child with `get_state`. See the `uds-protocol`
doc (`docs {"name":"uds-protocol"}`) for the wire shape.

## Sessions

Each subagent gets its own session, persisted under `<base_dir>/sessions/`.

- **Default display label**: `subagent` (if no `agent_id` is provided)
- **Custom display label**: The `agent_id` value is a user-facing label, not durable identity
- **Session UUID**: each spawn mints and returns a UUID used for `agent_cmd` targeting, the child session, registry, socket bookkeeping, and monitor/reaper keys

### Display label validation

Spawn display labels must contain only alphanumeric characters, hyphens, and underscores
(`[a-zA-Z0-9_-]`, 1–64 characters). The following are rejected:

- Path traversal attempts (`../../tmp/evil`)
- Spaces or special characters
- Empty strings or strings longer than 64 characters

The same validation is applied in both `spawn` and `agent_cmd`.

## Agent ID allowlists

The spawn tool supports an allowlist of permitted agent IDs. When configured:

- Only IDs in the allowlist can be spawned
- Requests with unlisted IDs are rejected with an error
- An empty allowlist (the default) permits any valid agent ID

Currently, the allowlist is always empty in the CLI agent, meaning any valid
agent ID is accepted. The allowlist mechanism exists for future integrations
that need to restrict which subagents can be spawned.

## Child process lifecycle

### Startup

1. `spawn` launches the child with `quecto agent --mode uds --socket <path> --spawned --parent-control <sidecar>` (no `--persist`, #1937)
2. Polls for socket readiness (100ms intervals, 10s timeout)
3. If the socket does not become ready, the child is terminated through the owned-child supervisor and an error is returned
4. Registers the child in the shared `SubagentRegistry` by UUID while retaining `agent_id` as the display label
5. If `task` was provided, sends it as the initial `prompt` via UDS (fire-and-forget)

#### Launch-bound parent control (#1935)

A launcher-created child is lifetime-scoped to the harness that launched it.
At launch the parent mints one random, generation-scoped capability for the
child and writes it to a private (`0600`, exclusively created) sidecar under
the runtime directory; only the sidecar *path* is passed on argv
(`--parent-control`), never the material, and nothing is put in the
environment. The child reads and removes the sidecar before it announces its
socket (a missing or malformed sidecar fails startup closed), then accepts
**exactly one** connection presenting that capability — the parent's monitor
connection, which sends the presentation as its first frame. A missing,
mismatched, replayed or second presentation closes that connection. Only the
loss (EOF/reset) of the bound connection runs the child's common shutdown
(reason `parent_connection_lost`: cancel the turn, ask its own direct
children to shut down over the protocol, persist, exit). Ordinary TUI,
inspector, tool or probe clients disconnecting — even the last one — never
end a launched child. The bound connection also carries the `BoundParent`
authority for the `shutdown` and `terminate_delegated_agent` commands.

Over the proxy (container) transport the same connection rides one bridged
proxy process whose stdin is the parent's pipe, so a parent that dies —
even by SIGKILL — closes the pipe, the proxy exits, and the child observes
its bound connection lost end to end. The capability is never forwarded to
the child's own children: each launch mints its own.

A launcher that dies *between* spawning the child and presenting the
capability would otherwise leave an unbound orphan (a launch-bound child
ignores client churn), so a launched child arms a **bind deadline** (30 s by default;
`QUECTO_PARENT_BIND_DEADLINE_MS` overrides it for tests): if no parent has
bound by then, the binding is spent and the same common shutdown runs with
reason `parent_never_bound`.

**Not yet covered (#1939):** the parent-loss / never-bound shutdown runs the
common teardown only — cancel the turn, ask direct children to shut down
over the protocol, persist, exit. It does **not** finalize script-managed
environments the way the SIGTERM path does (which additionally runs
`delete_all_subagents_from_registry`, i.e. each environment's retained
`kill`). A child that owns container environments and loses its parent
therefore leaves those containers running until #1939 moves environment
finalization into the application-owned teardown.

#### Lifetime and session restore (#1937)

A launcher-created child is **launch-bound**, not persistent. Its argv
carries no `--persist`; the harness resolves one lifetime at startup:

| Harness | Lifetime |
| --- | --- |
| Top-level, default | Exits when the last client disconnects |
| Top-level `--persist` (user-started, e.g. the TUI's tab agent or `quecto agent --mode uds --persist`) | Stays alive across client churn until SIGTERM/SIGINT or a protocol shutdown — unchanged |
| Launcher-created (`--parent-control`) | Ignores client churn; ends on loss of the bound parent connection or at the bind deadline. `--persist` is refused at startup (after the sidecar is consumed) |

Because a child cannot outlive its launcher, **session restore never
readopts children**. `resume_session` (and a new harness loading a saved
session) restores the transcript, the workflow run and past child messages,
and resets the operational roster to empty: persisted roster rows — live,
detached, dead, explicitly killed or malformed — are read only to be
ignored. No socket is probed, no pid compared, no monitor created. The
legacy `socketPath`/`pid` fields are no longer written; a legacy record is
migrated without them on its next save. The master explicitly re-spawns the
workers it needs; each re-spawn mints a fresh UUID and launch generation.
Nothing restarts automatically, and no historical roster row is shown as
live. Externally attached clients reconnecting to a still-running harness
see that harness's in-memory registry as before.

**Switching sessions releases the current session's launched children
through parent loss.** No later session can readopt them, so
`resume_session` into another session and `new_session` do not merely drop
their rows: each departing row's monitor task — the owner of the child's
bound parent-control connection — and its proxy bridge (container
transport) are aborted *before* the roster is cleared. The child observes
the loss of its bound parent and runs its own graceful shutdown (the #1935
parent-loss path: cancel the turn, ask its children to shut down, persist,
exit, remove its socket). No signal is sent and the dispatch path does not
wait; the reaper observes the exit and retires the supervisor handle.
Re-spawn the workers you need in the new session. (Interim until #1938
replaces this release with the acknowledged session-transition teardown —
which is also where script-managed environments of a departing child get
finalized; see #1939.)

### Running

- The child runs independently as a background process
- The parent continues its agent loop and can spawn additional children
- The parent interacts with children via `agent_cmd` (native UDS, no subprocess)
- Multiple children can run concurrently

### Cleanup

- **Owned-child supervisor** (#1935): every process this harness spawns
  directly (local launches and per-connection proxy bridge processes) is
  spawned and reaped by one `OwnedChildSupervisor` on its own runtime.
  Callers hold an opaque handle, never a pid or a `Child`. The reaper task
  takes its exit signal from the supervisor and removes the registry entry
  when the child exits
- **Explicit shutdown**: `shutdown_all()` asks the supervisor to terminate
  every locally launched child: the `shutdown` protocol command is always
  attempted first over the child's endpoint; only a negative outcome
  (unreachable, refused, no ACK within 5 s, or no exit within 10 s after an
  ACK) authorises SIGTERM to the retained handle (and its own process group
  where one was created), then SIGKILL after a 2 s grace. Each signal kind
  is sent at most once per handle and never after the reap. Script and
  container members hold no local handle and have no host signal fallback.
  The #1925 *reported same-namespace* lease still covers descendant rows
  reported by a host-local child until #1940 retires it; restored,
  container-reported and fixture-built rows are never signalled
- **Socket cleanup**: Socket files are removed by the child's UDS server on exit.
  Dead auto-generated sockets are reaped by liveness check on next agent startup; the 24h age threshold is a fallback when liveness cannot be determined

### Duplicate prevention

Spawning with a display label that is already live returns an error:

```
Failed to spawn subagent: duplicate live subagent display label 'worker-1'
```

Wait for the existing agent to finish (check with `agent_cmd get_state`) or
`abort` it before spawning a new one with the same ID.

## Disabling subagents

To prevent the LLM from spawning subagents entirely, disable and hide both tools
before the session starts: `--disable-tool spawn --disable-tool agent_cmd`. The
same `--disable-tool` flag works for any core or extension tool name on a
top-level agent; on a child, use spawn `disable_tools` / `read_only` (above).

## Interactive interfaces

The setup/configuration REPL does not operate agents or subagents. Use `quecto agent`, UDS, or `quecto-tui` for subagent capabilities.

For swarm control receipts, pause/resume behavior, observed budgets and retained
export scope, see [swarm operational diagnostics](swarm.md#operational-diagnostics-and-budgets).
