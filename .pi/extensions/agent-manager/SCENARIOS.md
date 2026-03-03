# Agent Manager Extension — BDD Scenarios

## Feature: Multi-repo headless agent management

### Background
The agent-manager extension spawns `pi --mode rpc` or `quecto agent --mode rpc` processes,
manages them via named FIFOs, parses their event streams, and provides a TUI dashboard.

---

### Scenario 1: Spawn a new agent
**Given** the user runs `/agents-spawn /path/to/repo`
**Then** a new RPC process is spawned at that path
**And** a FIFO is created for its stdin
**And** the agent appears in the widget with status `starting`

### Scenario 2: Widget shows all agents at a glance
**Given** three agents are running with statuses `running`, `idle`, `blocked`
**When** the widget renders
**Then** each agent row shows: status icon · label · issue/task · workflow progress bar · last action
**And** the header shows summary counts

### Scenario 3: Status transitions from RPC events
**Given** an agent is `idle`
**When** a `turn_start` event is received
**Then** the agent status becomes `running`
**When** an `agent_end` event is received
**Then** the agent status becomes `idle`
**When** a `tool_execution_end` event with `isError: true` is received
**Then** the agent status becomes `error`

### Scenario 4: lastToolCall updated from tool_execution_start
**Given** an agent is running
**When** a `tool_execution_start` event arrives with toolName `bash` and args `{command: "cargo test"}`
**Then** `agent.lastToolCall` is set to `"bash: cargo test"`

### Scenario 5: lastText extracted from agent_end
**When** an `agent_end` event arrives with messages containing a final assistant message
**Then** `agent.lastText` is set to the last assistant text snippet

### Scenario 6: Workflow state parsed from session JSONL
**Given** a session file containing toolResult entries with toolName `workflow`
**When** `parseWorkflowState(filePath)` is called
**Then** it returns the latest `steps` array and `activeIssue`

### Scenario 7: Open full dashboard with /agents command
**When** user types `/agents`
**Then** a full-screen TUI dashboard opens with agent tabs
**And** pressing `Esc` closes it and returns to normal TUI

### Scenario 8: Send a steer message from the dashboard
**Given** an agent tab is focused in the dashboard
**When** the user presses `S` and types a message
**Then** `{"type":"steer","message":"..."}` is written to the agent's FIFO

### Scenario 9: Abort an agent from the dashboard
**Given** an agent tab is focused in the dashboard
**When** the user presses `A`
**Then** `{"type":"abort"}` is written to the agent's FIFO

### Scenario 10: Blocked alert fires notification
**When** an agent remains `idle` with incomplete workflow steps for >5 minutes
**Then** `ctx.ui.notify()` is called with a `⚠` message
**And** the agent is marked with a `⚠` in the widget

### Scenario 11: agent_manager tool for LLM orchestration
**When** the LLM calls `agent_manager({action: "status"})`
**Then** a text summary of all agents is returned

### Scenario 12: LLM can spawn agent via tool
**When** the LLM calls `agent_manager({action: "spawn", cwd: "/path/to/repo"})`
**Then** a new RPC process is spawned and registered

### Scenario 13: State persists via appendEntry
**When** an agent is spawned or updated
**Then** `pi.appendEntry("agent-manager-state", {...})` is called
**And** on next `session_start`, the state is reconstructed

### Scenario 14: Graceful shutdown
**When** `session_shutdown` fires
**Then** `{"type":"abort"}` is sent to each agent
**And** all FIFOs are closed
**And** holder processes are killed

### Scenario 15: Tab navigation in dashboard
**When** the dashboard is open
**Then** `Tab`/`]` cycles to next agent
**And** `Shift+Tab`/`[` cycles to previous agent

---

## Feature: Cron Heartbeat Extension

### Scenario 16: Heartbeat fires every 5 minutes
**Given** the heartbeat is enabled with default interval
**When** 5 minutes pass
**Then** `pi.sendUserMessage(heartbeatPrompt, {deliverAs: "followUp"})` is called

### Scenario 17: Heartbeat skips if pending messages
**Given** the heartbeat is enabled
**When** the tick fires but `ctx.hasPendingMessages()` returns true
**Then** the tick is skipped silently

### Scenario 18: Heartbeat uses followUp delivery
**When** the agent is streaming and a tick fires
**Then** the message is delivered as `followUp` (never interrupts)

### Scenario 19: Heartbeat status shown in footer
**When** heartbeat is enabled
**Then** `ctx.ui.setStatus("heartbeat", "♥ heartbeat  next in Xm Ys")` is updated each second

### Scenario 20: /heartbeat-on enables timer
**When** user types `/heartbeat-on`
**Then** the interval timer starts

### Scenario 21: /heartbeat-off disables timer
**Given** heartbeat is enabled
**When** user types `/heartbeat-off`
**Then** the timer is cleared
**And** the footer status is cleared

### Scenario 22: /heartbeat-config sets interval and prompt
**When** user types `/heartbeat-config`
**Then** a dialog asks for interval in minutes and optional custom prompt

### Scenario 23: /heartbeat-now fires immediately
**When** user types `/heartbeat-now`
**Then** the heartbeat fires immediately regardless of timer

### Scenario 24: State persists across session restart
**Given** heartbeat was enabled with interval 3m
**When** the session is restarted
**Then** `enabled: true` and `intervalMs: 180000` are reconstructed from session entries
**And** the timer restarts automatically

### Scenario 25: Ctrl+Shift+H toggles heartbeat
**When** user presses `Ctrl+Shift+H`
**Then** heartbeat is toggled on/off
