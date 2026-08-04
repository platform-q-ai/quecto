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
- after a child has exited, deliberately reusing the same `agent_id` display label starts a fresh child session under a new hidden identity; use `get_messages` before cleanup if you need the previous result.

Do not reuse stale child context merely to avoid a new session; use it only
when its prior context is relevant and safe for the new assignment.

### Non-blocking execution and result recovery

`spawn` returns when the child **socket** is ready — not when the task finishes.
Completion is **multi-turn**. The production `agent_cmd` tool schema currently
**hides** blocking `await` from the model (`AWAIT_VISIBLE_IN_SCHEMA = false` in
`agent_cmd.rs`); dispatch still accepts it if invented. Prefer the passive path.

#### Required sequence

1. **Spawn** (and brief the child). Returns immediately.
2. **End this parent turn** (or do other *non-duplicative* work that does not need
   the child’s answer). Stay available to the user.
3. **Next turn:** a passive one-line completion note arrives automatically when
   the child finishes/errors/exits.
4. **Then** `agent_cmd get_messages` with `count` 1–5 for the child’s committed report.
5. Verify, synthesize, and answer the user. Relay conclusions — not raw child dumps
   unless asked.

#### Do not

- Poll `get_subagents`, `get_subagents_all`, or `get_state` in a loop waiting for idle.
- `sleep` / bash-wait / busy-wait for the child in the same turn.
- Treat the passive note as the child’s report — always `get_messages` for content.

#### Optional tools (not wait loops)

- `get_state` — occasional live progress/debug.
- `get_subagents_all` — inventory and cleanup **after** coordination, not completion waiting.
- `abort` / `kill` — stop work or the process when needed.

If you need the child’s answer before you can help the user, **yield the turn**
and continue when the note arrives; do not invent a same-turn wait.

(Sections below that document `await` describe the still-implemented command for
operators/tests and for flipping `AWAIT_VISIBLE_IN_SCHEMA` back on — not the
default agent-facing path.)

### Safety for delegated work

- Children inherit the parent's sandbox posture, credentials, and tools. Do not
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
process in UDS mode (`--mode uds --persist`). The child process:

- Uses the same quecto binary (`std::env::current_exe()`)
- Inherits the parent's `QUECTO_BASE_DIR` (config, credentials, sessions)
- Inherits the parent's sandbox posture (`--no-sandbox`)
- Gets its own hidden session identity minted per spawn; `agent_id` remains the display label used by parent tools for live subagents
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
      "description": "Display label for the subagent (used to address live subagents via agent_cmd)"
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
      "description": "Model for the child in provider/model form (e.g. 'openai/gpt-5.5'), same format as agent_cmd set_model. Forwarded as --model at launch so the child's first turn runs on it"
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
- **`agent_id`** is a display label and must be unique among live subagents. Spawning with an already-live label returns an error; reusing it after exit starts a fresh hidden identity.
- Returns immediately (< 1 second) after the child's socket is ready.
- **`workflow_spec` vs `workflow`.** `workflow: true` makes the workflow tool available so the *child* picks a template; `workflow_spec` hands the child a specific template **by value** and binds it. They are independent of `config`, which supplies the child's runtime (providers/model/default template library).
- **`model` (optional).** Sets the child's model at launch — accepts either a full `provider/model` string (e.g. `openai/gpt-5.5`) or a `provider` + `model_id` pair, the same format(s) as `agent_cmd set_model` (and validated by the same logic). It is forwarded to the child as `--model`, so the child's **first turn** (if `task` is given) already runs on the chosen model — no follow-up `set_model` round-trip needed. **Precedence:** an explicit `model` arg wins over any model from a forwarded `--config`, which wins over the built-in default. An invalid combination (e.g. `provider` without `model_id`) is a clear spawn error rather than a silent fall-back to the default.
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
accidental writes, not an isolation boundary. For stronger guarantees use a
workspace/sandbox posture. For a top-level agent, the CLI equivalent is
`--disable-tool` (repeatable; see the README).

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
  child's execution phase, current/recent tool activity, accurate canonical
  message count, model/effort, and workflow snapshot (mode, current step,
  done/total) on demand — including while the child is mid-turn;
- in the TUI, the selected child renders its own workflow status bar.

Read the child's final result the usual way — its one-line auto-note at your next
idle turn, then `agent_cmd get_messages` for the full
output (see [Notification model](#notification-model)).

### `agent_cmd` — interact with a subagent

```json
{
  "type": "object",
  "properties": {
    "agent_id": {
      "type": "string",
      "description": "Display label of a live spawned subagent"
    },
    "command": {
      "type": "string",
      "enum": ["prompt", "steer", "follow_up", "abort", "kill", "await",
               "get_state", "get_messages",
               "get_session_stats", "get_subagents", "get_tool_catalogue",
               "list_tools", "set_model", "set_effort", "clear_history"],
      "description": "Command to send"
    },
    "message": {
      "type": "string",
      "description": "Message for prompt/steer/follow_up commands"
    },
    "count": {
      "type": "integer",
      "description": "Number of messages for get_messages (omit for the newest history page; N for the last N)"
    },
    "before": {
      "type": "string",
      "description": "Paging cursor for get_messages: a message id from a prior response's before field; returns the adjacent older page"
    },
    "timeout": {
      "type": "integer",
      "description": "Max seconds for await (default: 300)"
    },
    "idle_timeout": {
      "type": "integer",
      "description": "Seconds agent must stay idle before await returns (default: 5). Set to 0 for immediate return."
    }
  },
  "required": ["agent_id", "command"]
}
```

**Supported commands:**

| Command | Description | Requires `message` |
|---------|-------------|--------------------|
| `prompt` | Send a task/message to the subagent | Yes |
| `steer` | Interrupt and redirect the agent (takes precedence over the workflow auto-continue nudge) | Yes |
| `follow_up` | Queue a message for after the current run | Yes |
| `abort` | Full stop: cancel the current run, kill in-flight tool/child processes, and suppress workflow auto-continue (does not resume) | No |
| `kill` | Terminate the subagent process (SIGTERM) | No |
| `await` | Block until the subagent reaches a terminal state | No |
| `get_state` | Inspect live/in-flight supervision state: phase, current/recent tools, progress, message count, model/effort, streaming, and workflow | No |
| `get_messages` | Inspect the stable committed transcript, normally after the turn ends (omit `count` for the newest page; pass `count` for the last N; pass `before` to page older history). A busy snapshot can lag the active turn | No |
| `get_session_stats` | Get token usage and cost | No |
| `get_subagents` | List subagents spawned by this agent | No |
| `get_tool_catalogue` / `list_tools` | Return rich tool catalogue snapshot | No |
| `set_model` | Change the LLM model | No |
| `set_effort` | Change the reasoning effort (`none`/`low`/`medium`/`high`/`xhigh`/`max`, validated against the child's active model; invalid values are rejected with the valid list) | No |
| `clear_history` | Clear conversation history | No |

**Examples:**

```json
{"name": "agent_cmd", "arguments": {"agent_id": "security-reviewer", "command": "get_state"}}
```

```json
{"name": "agent_cmd", "arguments": {"agent_id": "security-reviewer", "command": "get_messages", "count": 3}}
```

```json
{"name": "agent_cmd", "arguments": {"agent_id": "security-reviewer", "command": "steer", "message": "Focus on auth vulnerabilities only"}}
```

```json
{"name": "agent_cmd", "arguments": {"agent_id": "security-reviewer", "command": "set_effort", "effort": "high"}}
```

## Notification model

There are two ways to learn that a child finished: a **non-blocking passive
auto-note** (the default) and a **blocking manual `await`**.

### Non-blocking: passive auto-notes (default)

Spawned agents are **auto-noted passively**. When a child reaches a terminal
state (completed / errored / exited) the parent automatically receives a single
**one-line completion note** — no `await` call required. The note is:

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

### Blocking: manual `await`

`await` is **optional**. Use it when you must **block synchronously** until the
child finishes *within the same turn* before continuing (see below). When you
`await` a completion, that completion's **duplicate auto-note is suppressed** —
you get the awaited result, not a redundant note for the same event. A later
re-run of the same child will auto-note again.

### When to use which

- **Default to the passive auto-note** — spawn, keep working, and react when the
  one-line note arrives at your next turn. This is best for fire-and-forget or
  parallel children whose results you don't need *right now*.
- **Use `await` only when a result gates your next step in the same turn** — for
  example a reviewer whose verdict you must read before continuing.

In both cases the note/await result is a **summary only**; to read the child's
full output call `get_messages` (optionally with `count` for the last N messages).
Intermediate child tool errors are the child's problem: they remain visible in
the child transcript/tool stream but do not interrupt the parent, set
`get_subagents.lastError`, or mark the child `status:error` unless the run later
emits a true terminal failure signal.

### What you can see without `await`

- **Workflow state changes** are forwarded onto the parent's event stream
  (identity-tagged with `agent_id` + `parent_id`). See "Observing the unit
  tree" below.
- **Live supervision** via `get_state` reports the child's current phase,
  tool activity, evidence-based recent progress, and canonical message count.
- **Stable transcript inspection** via `get_messages` is intended for committed
  or end-of-turn output. A busy response is a snapshot and can lag mutable
  in-flight transcript content.

Neither command registers as a notification. A single call that catches the
agent mid-run tells you nothing about what happens next.

### The auto-note is a summary, not the result

The one-line completion note tells you *that* a child finished and gives a brief
outcome — it does **not** contain the child's full output. If you care about the
result, read it explicitly:

```json
// 1. Spawn — the child is auto-awaited from here on.
{"name": "spawn", "arguments": {"agent_id": "worker", "task": "do the thing"}}

// 2. (do other work) — at your next idle turn you receive, automatically:
//    Agent 'worker' completed and is ready for inspection

// 3. Inspect the full output when the note tells you the child is done.
{"name": "agent_cmd", "arguments": {"agent_id": "worker", "command": "get_messages", "count": 5}}
```

**Common mistake:** treating the one-line note as the child's result. Always
tail the output to see what the agent actually produced.

### Canonical pattern (blocking): await + tail

When you must not continue until the child has finished — for example a reviewer
whose verdict gates the next step — `await` it, then read the tail with `get_messages`:

```json
// 1. Spawn
{"name": "spawn", "arguments": {"agent_id": "worker", "task": "do the thing"}}

// 2. Block until idle, exited, error, or timeout
{"name": "agent_cmd", "arguments": {"agent_id": "worker", "command": "await", "timeout": 60}}

// 3. Inspect output (check the actual result or error)
{"name": "agent_cmd", "arguments": {"agent_id": "worker", "command": "get_messages", "count": 5}}
```

`await` reports *that* something happened (idle/exited/error/timeout) but not
*what*; `get_messages` (with `count`) shows the final assistant message or error details.

### `await` — block until a subagent finishes

The `await` command blocks the calling tool until the target subagent reaches
a terminal state — idle, exited, or timeout. This eliminates the need for
polling loops that burn LLM tokens.

**Parameters:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `timeout` | integer (seconds) | 300 | Max wall-clock wait time |
| `idle_timeout` | integer (seconds) | 5 | Seconds the agent must stay idle before returning. Resets if the agent resumes streaming (e.g. auto-continue between workflow steps). Set to 0 for immediate return on first idle. |

**Return value (structured JSON):**

```json
{
  "status": "idle",
  "reason": "completed",
  "agent_id": "bookmarks-v1",
  "elapsed_ms": 47200,
  "workflow": {
    "mode": "complete",
    "steps_completed": 7,
    "steps_total": 7
  }
}
```

| Status | Reason | Description |
|--------|--------|-------------|
| `idle` | `idle` | Agent stayed idle for the full `idle_timeout` window (or immediately when `idle_timeout: 0`) |
| `exited` | `exit_code_0` | Process exited cleanly |
| `exited` | `exit_code_<N>` | Process exited with error code N |
| `exited` | `signal_<N>` | Process killed by signal N |
| `timeout` | `null` | Wall-clock `timeout` exceeded |
| `error` | `agent_not_found` | Agent ID not in registry |
| `error` | `connection_failed` | Socket exists but connection refused |
| `error` | `another_await_active` | Another `await` is already waiting on this agent |

> The `status`/`reason` fields above describe the await **lifecycle** (how the
> wait ended). They are distinct from the typed **verdict** in `result.status`
> (`completed` / `incomplete` / `failed` / `running`), which is what a parent
> branches on. An `idle` lifecycle yields a `completed` verdict only when the
> agent's workflow actually reached `complete`; otherwise the verdict is
> `incomplete`. So a finished idle agent returns `reason: "idle"`, not
> `reason: "completed"`.

**Examples:**

```json
{"name": "agent_cmd", "arguments": {"agent_id": "reviewer", "command": "await", "timeout": 600}}
```

```json
{"name": "agent_cmd", "arguments": {"agent_id": "reviewer", "command": "await", "idle_timeout": 0}}
```

**Key behaviors:**

- **Auto-continue safe:** The `idle_timeout` window correctly filters brief
  idle gaps between auto-continue workflow steps.
- **One awaiter per agent:** Only one `await` can be *in flight* per agent at a
  time. A second **concurrent** `await` — from another connection/caller or a
  racing turn — returns `"another_await_active"` immediately. Note that multiple
  `await`s issued as sibling tool calls within a single turn are executed
  **sequentially** by the agent loop, so they do not overlap and each succeeds in
  turn; you only see `"another_await_active"` when two awaiters genuinely race the
  same agent.
- **Interacts with abort/steer/kill:** `abort` and `steer` do not interrupt
  `await` — it continues waiting. `kill` causes `await` to return with
  `"exited"` status.
- **Workflow snapshot:** The `workflow` field is a read-only snapshot of
  workflow state at the moment of return (null if workflow is not enabled).

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
- **Hidden session identity**: each spawn mints a fresh hidden UUID used for the child session, registry, socket bookkeeping, and monitor/reaper keys
- Reusing a display label after the previous child exits starts a clean session under a new hidden identity

### Session name validation

Agent IDs must contain only alphanumeric characters, hyphens, and underscores
(`[a-zA-Z0-9_-]`, 1–64 characters). The following are rejected:

- Path traversal attempts (`../../tmp/evil`)
- Spaces or special characters
- Empty strings or strings longer than 64 characters

The same validation is applied in both `spawn` and `agent_cmd`.

## Sandbox inheritance

The child inherits the parent's security posture:

| Parent flag | Child behavior |
|------------|---------------|
| `--no-sandbox` active | Child gets `--no-sandbox` (unrestricted file access) |
| `--no-sandbox` not set | Child uses default workspace restriction from config |

This ensures consistent security boundaries across the agent hierarchy. A
child agent cannot escalate its own privileges beyond what the parent has.

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

1. `spawn` launches the child with `quecto agent --mode uds --socket <path> --persist`
2. Polls for socket readiness (100ms intervals, 10s timeout)
3. If the socket does not become ready, the child is killed and an error is returned
4. Registers the child in the shared `SubagentRegistry` by hidden UUID while retaining `agent_id` as the display label
5. If `task` was provided, sends it as the initial `prompt` via UDS (fire-and-forget)

### Running

- The child runs independently as a background process
- The parent continues its agent loop and can spawn additional children
- The parent interacts with children via `agent_cmd` (native UDS, no subprocess)
- Multiple children can run concurrently

### Cleanup

- **Background reaper**: Each child has a `tokio::spawn` reaper task that calls
  `child.wait()` and removes the registry entry when the child exits
- **Explicit shutdown**: `shutdown_all()` sends SIGTERM to all tracked children
  and clears the registry
- **Socket cleanup**: Socket files are removed by the child's UDS server on exit.
  Stale sockets older than 24h are reaped on next agent startup

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

## REPL subagent commands

The interactive REPL provides commands for managing subagents:

| Command | Description |
|---------|-------------|
| `/spawn <task>` | Spawn a task as a child agent |
| `/spawn --agent <id> <task>` | Spawn with a specific agent ID |
| `/spawn --system <prompt> <task>` | Spawn with a custom system prompt |
| `/spawn --model <model> <task>` | Spawn with a specific model |
| `/spawn --max-time <secs> <task>` | Spawn with a wall-clock timeout |
| `/spawn --help` | Show spawn command help |
| `/agent list` | List available agent profiles |
| `/agent create <name>` | Create a new agent profile |
| `/agent show <name>` | Show an agent profile's configuration |
| `/agent edit <name>` | Edit an agent profile |
| `/agent remove <name>` | Remove an agent profile |
| `/agent run <name> <task>` | Run a task using an agent profile |

### TUI effort selector for sub-agents

In the TUI, the effort selector (`/effort <level>` to set directly, or bare `/effort` to open the
overlay) targets the **currently focused/selected sub-agent** when one is
active, instead of the primary session. The selected level is validated against
that child's agent-reported effort vocabulary and sent over the same UDS runtime
path as `agent_cmd set_effort` (`set_effort`); the footer only updates after the
child acknowledges the change, so a rejected level visibly keeps the previous
one. A child's session-scoped effort is reset to its own default when its
session is reset/resumed (`reset_effort_to_default`). With no sub-agent focused,
`/effort` continues to control the primary session.

## Architecture

```
Parent Agent Process
  │
  ├── LLM calls spawn(task="review code", agent_id="reviewer")
  │
  ├── SpawnTool::execute()
  │     ├── Validates agent_id format + allowlist
  │     ├── Rejects if the display label is already live
  │     ├── Launches: quecto agent --mode uds --socket <path> --persist
  │     ├── Polls for socket readiness (up to 10s)
  │     ├── Registers in SubagentRegistry (hidden UUID → socket_path + PID + display label)
  │     ├── Sends initial task as UDS prompt (if provided)
  │     ├── Spawns background reaper task
  │     └── Returns immediately: "Subagent 'reviewer' is running."
  │
  ├── LLM calls agent_cmd(agent_id="reviewer", command="get_state")
  │
  ├── AgentCmdTool::execute()
  │     ├── Validates agent_id format
  │     ├── Resolves the live display label to a hidden UUID registry entry
  │     ├── Connects to UDS socket
  │     ├── Sends JSON command, reads response (300s timeout)
  │     └── Returns structured response to LLM
  │
  └── Child Agent Process (reviewer)
        ├── Listening on /run/user/1000/quecto-agent-<agent-uuid>.sock
        ├── Loads config from QUECTO_BASE_DIR/config.json
        ├── Has its own LLM context (no shared state with parent)
        ├── Processes prompts via UDS protocol
        └── Exits when no more work / on SIGTERM
```

### Key design decisions

1. **Non-blocking spawn**: The parent returns in < 1 second. Three spawns
   that each take 60s cost ~3s setup time, then all run concurrently.

2. **Native UDS interaction**: `agent_cmd` connects to child sockets directly
   from Rust. No `ncat`, `socat`, or `bash` subprocess. Works in sandboxed
   environments where external tools may not be available.

3. **Shared registry**: `SubagentRegistry` (`Arc<Mutex<HashMap>>`) maps
   hidden UUID to socket path + PID + display label. Shared between `spawn` and
   `agent_cmd` via `Arc`; `agent_cmd` resolves live display labels through the
   registry before connecting. Entries are auto-removed when children exit.

4. **Process isolation**: Each subagent is a separate OS process. There is no
   shared memory, no shared LLM context, and no shared tool state.

5. **Config inheritance**: The child re-reads `config.json` from
   `QUECTO_BASE_DIR`. Config changes between parent startup and child spawn
   are visible to the child.

6. **Grandchildren**: A child agent can itself call `spawn`, creating a tree
   of processes. Each level inherits the same sandbox/network posture.

## Practical patterns

### Parallel code review (prefer passive notes)

Spawn multiple reviewers with distinct ownership; prefer passive completion
notes so the parent stays available:

```
"Spawn three read_only subagents for parallel review (distinct dimensions):
1. spawn(agent_id='arch-review', read_only=true, task='Review src/ for architecture issues; return concise findings with file:line')
2. spawn(agent_id='security-review', read_only=true, task='Review src/ for security issues; return concise findings with file:line')
3. spawn(agent_id='perf-review', read_only=true, task='Review src/ for performance issues; return concise findings with file:line')

Continue useful non-duplicative parent work. When each one-line completion note
arrives, read that child's report:
  agent_cmd(agent_id='…', command='get_messages', count=5)

Synthesize, dedupe, and judge before answering the user. Prefer passive notes; yield the turn if a
verdict must gate the next action in the same turn."
```

All three run concurrently — total time is the max of the three, not the sum.
Default to passive notes and the multi-turn sequence (see
[Notification model](#notification-model) and [Parent coordination model](#parent-coordination-model)).

### Delegating a bound workflow (and nesting)

Hand a child a specific process to follow with `workflow_spec`, and let it
delegate further:

```
"Spawn agent_id='pr-reviewer' with task='Review PR #682' and a workflow_spec bound
to a review-pr template (analyze the diff → run the tests → report). The child runs
exactly those steps in Active mode. Watch its workflow_state on your stream; when it
finishes, read the verdict with agent_cmd get_messages."
```

A workflowed child can itself spawn sub-agents bound to *their* own
`workflow_spec` — e.g. the review step spawns independent per-dimension reviewers,
each bound to a single-dimension review workflow. The whole tree stays
observable: every descendant's `workflow_state` is forwarded up your event stream
tagged with that agent's id, and the TUI shows each agent's own workflow bar.

### Fire-and-forget with auto-await

```
"Spawn agent_id='researcher' with task='Analyze the codebase and write findings to /tmp/report.md'.
Continue working on other tasks. You'll automatically get a one-line note when the
researcher finishes; then read the report with agent_cmd get_messages."
```

### Idle agents with on-demand prompts

```
"Spawn agent_id='helper' with no task (starts idle).
Later, when I need help:
  agent_cmd(agent_id='helper', command='prompt', message='Explain this function...')
  agent_cmd(agent_id='helper', command='get_messages', count=1)"
```

### Steering a running agent

```
"The security reviewer is taking too long on low-priority files.
  agent_cmd(agent_id='security-review', command='steer', message='Skip test files, focus on src/auth/ only')
"
```

### Aborting a stuck agent

`abort` is a **full stop**: it cancels the current turn, terminates any in-flight
tool call and its child processes (e.g. a long-running `bash`), discards queued
work, and suppresses workflow auto-continue so a workflow-bound agent does **not**
resume afterward. It stays stopped until you re-drive it with a fresh `prompt`.
The three control verbs stay distinct — `follow_up` = queue, `steer` = redirect,
`abort` = stop.

```
"The researcher seems stuck.
  agent_cmd(agent_id='researcher', command='abort')
"
```

## See also

- UDS Protocol Reference (`docs {"name":"uds-protocol"}`) — the framed JSON protocol used for agent communication
- Extensions (`docs {"name":"extensions"}`) — adding custom tools to the agent
