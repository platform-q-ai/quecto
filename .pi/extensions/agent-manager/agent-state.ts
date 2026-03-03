/**
 * Agent Manager — type definitions and pure state helpers.
 *
 * All functions here are side-effect-free. They take immutable input and
 * return new objects — no mutations, no I/O. This makes them trivially
 * testable without a pi runtime.
 */

// ─── Types ────────────────────────────────────────────────────────────────

export type AgentStatus = "starting" | "idle" | "running" | "blocked" | "error" | "done";
export type AgentType = "pi" | "quecto";

export interface WorkflowStep {
	id: number;
	done: boolean;
	label?: string;
}

export interface ActiveIssue {
	number: number;
	title: string;
}

export interface UsageStats {
	input: number;
	output: number;
	cacheRead: number;
	cacheWrite: number;
	cost: number;
}

export interface Alert {
	id: string;
	timestamp: number;
	message: string;
	read: boolean;
}

export interface ManagedAgent {
	id: string;
	label: string;
	cwd: string;
	sessionFile: string;
	agentType: AgentType;
	pid: number;
	holderPid: number;
	fifoPath: string;
	status: AgentStatus;
	workflowSteps?: WorkflowStep[];
	workflowIssue?: ActiveIssue;
	lastToolCall?: string;
	lastText?: string;
	usage: UsageStats;
	model?: string;
	alerts: Alert[];
	lastHeartbeat?: number;
}

export interface StatusSummary {
	running: number;
	idle: number;
	blocked: number;
	error: number;
	starting: number;
	done: number;
	total: number;
}

// ─── RPC event shapes (common subset: pi + quecto) ────────────────────────

export type RpcEvent =
	| { type: "agent_start" }
	| { type: "agent_end"; messages: Array<{ role: string; content: string | Array<{ type: string; text?: string }> }> }
	| { type: "turn_start" }
	| {
			type: "turn_end";
			message: { role: string; content: string; usage?: unknown; stopReason?: string };
			toolResults: unknown[];
	  }
	| { type: "tool_execution_start"; toolCallId: string; toolName: string; args: Record<string, unknown> }
	| { type: "tool_execution_end"; toolCallId: string; toolName: string; result: unknown; isError: boolean }
	| { type: "response"; command: string; success: boolean; data?: unknown; error?: string };

// ─── formatLastToolCall ────────────────────────────────────────────────────

const MAX_TOOL_LABEL = 70;

/**
 * Produce a short human-readable label for a tool execution, e.g. `"bash: cargo test"`.
 */
export function formatLastToolCall(toolName: string, args: Record<string, unknown>): string {
	let detail: string;
	switch (toolName) {
		case "bash":
			detail = String(args.command ?? "");
			break;
		case "read":
		case "write":
		case "edit":
			detail = String(args.path ?? args.file_path ?? "");
			break;
		case "ls":
			detail = String(args.path ?? ".");
			break;
		case "grep":
		case "find":
			detail = String(args.pattern ?? args.query ?? "");
			break;
		default:
			detail = Object.values(args)
				.slice(0, 1)
				.map((v) => String(v))
				.join("");
	}
	const full = `${toolName}: ${detail}`;
	if (full.length > MAX_TOOL_LABEL) {
		return full.slice(0, MAX_TOOL_LABEL - 1) + "…";
	}
	return full;
}

// ─── applyRpcEvent ─────────────────────────────────────────────────────────

/**
 * Immutably apply an RPC event to an agent, returning the updated agent.
 */
export function applyRpcEvent(agent: ManagedAgent, event: RpcEvent): ManagedAgent {
	switch (event.type) {
		case "agent_start":
			return { ...agent, status: "running" };

		case "agent_end": {
			// Extract lastText from the final assistant message
			let lastText: string | undefined;
			const msgs = event.messages;
			for (let i = msgs.length - 1; i >= 0; i--) {
				const msg = msgs[i];
				if (msg.role === "assistant") {
					if (typeof msg.content === "string") {
						lastText = msg.content;
					} else if (Array.isArray(msg.content)) {
						const textParts = msg.content
							.filter((p) => p.type === "text" && p.text)
							.map((p) => p.text!);
						lastText = textParts.join("\n") || undefined;
					}
					break;
				}
			}
			return { ...agent, status: "idle", lastText: lastText ?? agent.lastText };
		}

		case "turn_start":
			return { ...agent, status: "running" };

		case "turn_end":
			// Status stays running (more turns may come); only update after agent_end
			return agent;

		case "tool_execution_start": {
			const lastToolCall = formatLastToolCall(event.toolName, event.args);
			return { ...agent, lastToolCall };
		}

		case "tool_execution_end":
			if (event.isError) {
				return { ...agent, status: "error" };
			}
			return agent;

		case "response":
			// Responses don't change agent status
			return agent;

		default:
			return agent;
	}
}

// ─── computeStatusSummary ──────────────────────────────────────────────────

export function computeStatusSummary(agents: ManagedAgent[]): StatusSummary {
	const summary: StatusSummary = {
		running: 0,
		idle: 0,
		blocked: 0,
		error: 0,
		starting: 0,
		done: 0,
		total: agents.length,
	};
	for (const a of agents) {
		summary[a.status]++;
	}
	return summary;
}

// ─── formatProgress ────────────────────────────────────────────────────────

/**
 * Render a text progress bar for workflow steps.
 *
 * @param steps  Workflow steps array (or undefined = not a quecto agent)
 * @param width  Number of characters wide the bar should be
 * @returns      String of `█` and `░` characters
 */
export function formatProgress(
	steps: Array<{ id: number; done: boolean }> | undefined,
	width: number,
): string {
	if (!steps || steps.length === 0) {
		return "░".repeat(width);
	}
	const done = steps.filter((s) => s.done).length;
	const total = steps.length;
	const filled = Math.round((done / total) * width);
	return "█".repeat(filled) + "░".repeat(width - filled);
}

// ─── statusIcon ────────────────────────────────────────────────────────────

export function statusIcon(status: AgentStatus): string {
	switch (status) {
		case "running":
			return "●";
		case "idle":
			return "○";
		case "blocked":
			return "⚠";
		case "error":
			return "✗";
		case "starting":
			return "…";
		case "done":
			return "✓";
	}
}
