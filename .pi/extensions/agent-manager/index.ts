/**
 * Agent Manager Extension
 *
 * Spawns, monitors, and triages multiple headless pi/quecto RPC processes
 * from a single controller pi TUI session. Each managed agent runs either
 * `pi --mode rpc` or `quecto agent --mode rpc`, with a named FIFO for stdin.
 *
 * Commands:
 *   /agents              — open full-screen TUI dashboard
 *   /agents-spawn <path> — spawn a new agent in <path>
 *   /agents-steer <id> <msg> — send steer message to agent by id
 *   /agents-status       — print status summary to chat
 *
 * Shortcut: Ctrl+Shift+M → open/close dashboard
 *
 * Tool for the LLM: `agent_manager`
 *
 * Architecture: each agent has a FIFO + `sleep infinity` holder process that
 * keeps the FIFO write-end open so the RPC process doesn't exit on EOF.
 * We write JSON lines to the FIFO; the RPC process reads from it via stdin.
 * We read the RPC process stdout to parse events.
 */

import { spawn } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { StringEnum } from "@mariozechner/pi-ai";
import type { ExtensionAPI, ExtensionContext } from "@mariozechner/pi-coding-agent";
import { Text } from "@mariozechner/pi-tui";
import { Type } from "@sinclair/typebox";
import {
	applyRpcEvent,
	computeStatusSummary,
	formatProgress,
	statusIcon,
	type ManagedAgent,
	type RpcEvent,
} from "./agent-state.js";
import { AgentDashboard, type DashboardAction } from "./dashboard.js";
import { parseWorkflowStateFromFile } from "./session-parse.js";
import { buildWidgetRenderer } from "./widget.js";

// ─── State ────────────────────────────────────────────────────────────────

/** Runtime registry: agentId → ManagedAgent */
const registry = new Map<string, ManagedAgent>();

/** Line buffers for each agent's stdout */
const lineBuffers = new Map<string, string>();

/** Rendered-state callback to request widget re-render */
let onStateChange: (() => void) | null = null;

/** Interval ID for blocked-agent detection */
let blockedCheckInterval: ReturnType<typeof setInterval> | null = null;

// ─── Helpers ──────────────────────────────────────────────────────────────

function generateId(): string {
	return `agent-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
}

function fifoPath(id: string): string {
	return path.join(os.tmpdir(), `pi-agent-manager-${id}.fifo`);
}

function writeToFifo(agent: ManagedAgent, command: Record<string, unknown>): void {
	const line = JSON.stringify(command) + "\n";
	try {
		fs.appendFileSync(agent.fifoPath, line, { encoding: "utf-8", flag: "a" });
	} catch {
		// FIFO may be closed if agent exited
	}
}

function updateAgent(id: string, updater: (a: ManagedAgent) => ManagedAgent): void {
	const a = registry.get(id);
	if (a) {
		registry.set(id, updater(a));
		onStateChange?.();
	}
}

function getAgentsList(): ManagedAgent[] {
	return Array.from(registry.values());
}

function resolveAgentId(idOrLabel: string): ManagedAgent | undefined {
	return registry.get(idOrLabel) ?? [...registry.values()].find((a) => a.label === idOrLabel);
}

// ─── Process management ────────────────────────────────────────────────────

/** Spawn a new RPC agent. Returns the agent id. */
async function spawnAgent(
	opts: {
		cwd: string;
		label?: string;
		agentType?: "pi" | "quecto";
		sessionFile?: string;
	},
	ctx: ExtensionContext,
): Promise<string> {
	const id = generateId();
	const fPath = fifoPath(id);
	const agentType = opts.agentType ?? "quecto";
	const label = opts.label ?? path.basename(opts.cwd);

	// Create FIFO
	try {
		await new Promise<void>((resolve, reject) => {
			spawn("mkfifo", [fPath]).on("close", (code) => {
				if (code === 0) resolve();
				else reject(new Error(`mkfifo failed with code ${code}`));
			});
		});
	} catch (err) {
		ctx.ui.notify(`Failed to create FIFO for agent ${label}: ${err}`, "error");
		throw err;
	}

	// Spawn holder: keeps FIFO write-end open (redirect its stdout to the FIFO
	// so the write-end remains open as long as the holder is alive)
	const holder = spawn("sh", ["-c", `exec > "${fPath}"; sleep infinity`], {
		cwd: opts.cwd,
		stdio: "ignore",
		detached: false,
	});

	// Spawn RPC process — read stdin from FIFO, pipe stdout for events
	const rpcArgs =
		agentType === "quecto"
			? ["agent", "--mode", "rpc", ...(opts.sessionFile ? ["-s", opts.sessionFile] : [])]
			: ["--mode", "rpc", ...(opts.sessionFile ? ["-s", opts.sessionFile] : [])];

	const rpcBin = agentType === "quecto" ? "quecto" : "pi";
	const proc = spawn(
		"sh",
		["-c", `exec < "${fPath}"; exec ${rpcBin} ${rpcArgs.map((a) => JSON.stringify(a)).join(" ")}`],
		{
			cwd: opts.cwd,
			stdio: ["ignore", "pipe", "pipe"],
			detached: false,
		},
	);

	const sessionFile =
		opts.sessionFile ??
		path.join(os.homedir(), ".pi", "agent", "sessions", `${id}.jsonl`);

	const agent: ManagedAgent = {
		id,
		label,
		cwd: opts.cwd,
		sessionFile,
		agentType,
		pid: proc.pid ?? 0,
		holderPid: holder.pid ?? 0,
		fifoPath: fPath,
		status: "starting",
		usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0 },
		alerts: [],
	};

	registry.set(id, agent);
	lineBuffers.set(id, "");
	onStateChange?.();

	// Load workflow state from existing session file
	parseWorkflowStateFromFile(sessionFile).then((state) => {
		if (state) {
			updateAgent(id, (a) => ({
				...a,
				workflowSteps: state.steps,
				workflowIssue: state.activeIssue,
			}));
		}
	});

	// Wire up stdout → event stream
	proc.stdout?.on("data", (data: Buffer) => {
		let buf = (lineBuffers.get(id) ?? "") + data.toString("utf-8");
		const lines = buf.split("\n");
		buf = lines.pop() ?? "";
		lineBuffers.set(id, buf);
		for (const line of lines) {
			processLine(id, line.trim());
		}
	});

	proc.on("close", () => {
		updateAgent(id, (a) => ({ ...a, status: a.status === "running" ? "idle" : a.status }));
	});

	ctx.ui.notify(`Spawned agent: ${label} (${agentType})`, "info");
	return id;
}

function processLine(agentId: string, line: string): void {
	if (!line) return;
	let event: RpcEvent;
	try {
		event = JSON.parse(line) as RpcEvent;
	} catch {
		return;
	}
	updateAgent(agentId, (a) => applyRpcEvent(a, event));
}

function killAgent(id: string): void {
	const agent = registry.get(id);
	if (!agent) return;

	// Send abort command
	try {
		writeToFifo(agent, { type: "abort" });
	} catch {
		// ignore
	}

	// Kill processes
	try {
		if (agent.pid) process.kill(agent.pid, "SIGTERM");
	} catch {
		// ignore
	}
	try {
		if (agent.holderPid) process.kill(agent.holderPid, "SIGTERM");
	} catch {
		// ignore
	}

	// Clean up FIFO
	try {
		fs.unlinkSync(agent.fifoPath);
	} catch {
		// ignore
	}

	registry.delete(id);
	lineBuffers.delete(id);
	onStateChange?.();
}

// ─── Blocked detection ────────────────────────────────────────────────────

const BLOCKED_THRESHOLD_MS = 5 * 60 * 1_000; // 5 minutes

function checkForBlockedAgents(ctx: ExtensionContext): void {
	const now = Date.now();
	for (const agent of registry.values()) {
		if (agent.status !== "idle") continue;
		const incompleteSteps = agent.workflowSteps?.some((s) => !s.done) ?? false;
		if (!incompleteSteps) continue;

		const lastActivity = agent.lastHeartbeat ?? 0;
		if (now - lastActivity < BLOCKED_THRESHOLD_MS) continue;

		// Mark blocked + alert
		updateAgent(agent.id, (a) => ({
			...a,
			status: "blocked",
			alerts: [
				...a.alerts,
				{
					id: `blocked-${now}`,
					timestamp: now,
					message: `Agent idle with incomplete workflow for >${Math.round(BLOCKED_THRESHOLD_MS / 60_000)}m`,
					read: false,
				},
			],
		}));
		ctx.ui.notify(`⚠ ${agent.label} blocked — open agents dashboard`, "warning");
	}
}

// ─── Persist / reconstruct state ──────────────────────────────────────────

interface PersistedAgentRecord {
	id: string;
	label: string;
	cwd: string;
	sessionFile: string;
	agentType: "pi" | "quecto";
	fifoPath: string;
}

function persistState(pi: ExtensionAPI): void {
	const records: PersistedAgentRecord[] = [...registry.values()].map((a) => ({
		id: a.id,
		label: a.label,
		cwd: a.cwd,
		sessionFile: a.sessionFile,
		agentType: a.agentType,
		fifoPath: a.fifoPath,
	}));
	pi.appendEntry("agent-manager-state", { agents: records });
}

function reconstructState(ctx: ExtensionContext): void {
	// Find last agent-manager-state custom entry
	const entries = ctx.sessionManager.getBranch();
	let lastRecord: PersistedAgentRecord[] | null = null;
	for (const entry of entries) {
		if (
			entry.type === "custom" &&
			entry.customType === "agent-manager-state" &&
			Array.isArray((entry as { data?: { agents?: PersistedAgentRecord[] } }).data?.agents)
		) {
			lastRecord = (entry as { data: { agents: PersistedAgentRecord[] } }).data.agents;
		}
	}
	if (!lastRecord) return;

	// Re-check which agents are still running
	for (const rec of lastRecord) {
		// Check if the RPC process is still alive (try sending null signal)
		let isAlive = false;
		try {
			if (rec.fifoPath && fs.existsSync(rec.fifoPath)) isAlive = true;
		} catch {
			// ignore
		}

		if (!isAlive) continue;

		const agent: ManagedAgent = {
			id: rec.id,
			label: rec.label,
			cwd: rec.cwd,
			sessionFile: rec.sessionFile,
			agentType: rec.agentType,
			pid: 0, // unknown after restart
			holderPid: 0,
			fifoPath: rec.fifoPath,
			status: "idle",
			usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0 },
			alerts: [],
		};
		registry.set(rec.id, agent);
	}
}

// ─── Dashboard handler ────────────────────────────────────────────────────

async function openDashboard(ctx: ExtensionContext, pi: ExtensionAPI): Promise<void> {
	if (!ctx.hasUI) {
		printStatusToChat(ctx);
		return;
	}

	const action = await ctx.ui.custom<DashboardAction>((_, theme, _kb, done) => {
		return new AgentDashboard(getAgentsList(), theme, done);
	});

	if (!action || action.action === "close") return;

	switch (action.action) {
		case "send": {
			const agent = registry.get(action.agentId);
			if (!agent) return;
			const cmd =
				action.as === "steer"
					? { type: "steer", message: action.message }
					: action.as === "followUp"
						? { type: "follow_up", message: action.message }
						: { type: "prompt", message: action.message };
			writeToFifo(agent, cmd);
			break;
		}
		case "abort": {
			const agent = registry.get(action.agentId);
			if (agent) writeToFifo(agent, { type: "abort" });
			break;
		}
		case "kill":
			killAgent(action.agentId);
			break;
		case "relaunch": {
			const agent = registry.get(action.agentId);
			if (agent) {
				killAgent(action.agentId);
				await spawnAgent({ cwd: agent.cwd, label: agent.label, agentType: agent.agentType, sessionFile: agent.sessionFile }, ctx);
				persistState(pi);
			}
			break;
		}
		case "spawn": {
			const cwd = await ctx.ui.input("Repo path:", "");
			if (cwd?.trim()) {
				await spawnAgent({ cwd: cwd.trim() }, ctx);
				persistState(pi);
			}
			break;
		}
	}
}

function printStatusToChat(ctx: ExtensionContext): void {
	const agents = getAgentsList();
	if (agents.length === 0) {
		ctx.ui.notify("No agents running. Use /agents-spawn <path> to start one.", "info");
		return;
	}
	const s = computeStatusSummary(agents);
	const lines = [
		`Agents: ${agents.length} total (${s.running} running, ${s.idle} idle, ${s.blocked} blocked, ${s.error} errored)`,
		...agents.map((a) => {
			const bar = formatProgress(a.workflowSteps, 10);
			const done = a.workflowSteps?.filter((s) => s.done).length ?? 0;
			const total = a.workflowSteps?.length ?? 16;
			const icon = statusIcon(a.status);
			return `  ${icon} ${a.label} [${a.status}] ${bar} ${done}/${total}${a.lastToolCall ? `  → ${a.lastToolCall}` : ""}`;
		}),
	];
	ctx.ui.notify(lines.join("\n"), "info");
}

// ─── Extension entry point ────────────────────────────────────────────────

export default function (pi: ExtensionAPI) {
	// ── Session lifecycle ──────────────────────────────────────────────

	pi.on("session_start", async (_event, ctx) => {
		reconstructState(ctx);
		refreshWidget(ctx);
		onStateChange = () => refreshWidget(ctx);
	});

	pi.on("session_switch", async (_event, ctx) => {
		// Clear and reconstruct for new session
		registry.clear();
		lineBuffers.clear();
		reconstructState(ctx);
		refreshWidget(ctx);
	});

	pi.on("session_shutdown", async (_event, _ctx) => {
		// Stop blocked-detection timer
		if (blockedCheckInterval !== null) {
			clearInterval(blockedCheckInterval);
			blockedCheckInterval = null;
		}
		// Gracefully shut down all agents
		for (const agent of registry.values()) {
			try {
				writeToFifo(agent, { type: "abort" });
			} catch {
				// ignore
			}
			try {
				if (agent.pid) process.kill(agent.pid, "SIGTERM");
			} catch {
				// ignore
			}
			try {
				if (agent.holderPid) process.kill(agent.holderPid, "SIGTERM");
			} catch {
				// ignore
			}
			try {
				fs.unlinkSync(agent.fifoPath);
			} catch {
				// ignore
			}
		}
		registry.clear();
	});

	// ── Periodic blocked-detection (every minute) ─────────────────────

	pi.on("session_start", async (_event, ctx) => {
		if (blockedCheckInterval !== null) clearInterval(blockedCheckInterval);
		blockedCheckInterval = setInterval(() => checkForBlockedAgents(ctx), 60_000);
	});

	// ── Widget ────────────────────────────────────────────────────────

	function refreshWidget(ctx: ExtensionContext): void {
		const renderer = buildWidgetRenderer(getAgentsList());
		ctx.ui.setWidget("agent-manager", renderer);
	}

	// ── Commands ──────────────────────────────────────────────────────

	pi.registerCommand("agents", {
		description: "Open full-screen Agent Manager dashboard",
		handler: async (_args, ctx) => {
			await openDashboard(ctx, pi);
		},
	});

	pi.registerCommand("agents-spawn", {
		description: "Spawn a new headless agent. Usage: /agents-spawn <path> [label] [pi|quecto]",
		handler: async (args, ctx) => {
			const parts = args.trim().split(/\s+/);
			const cwd = parts[0];
			if (!cwd) {
				ctx.ui.notify("Usage: /agents-spawn <path> [label] [pi|quecto]", "warning");
				return;
			}
			const label = parts[1];
			const agentType = (parts[2] === "pi" || parts[2] === "quecto") ? parts[2] : undefined;
			await spawnAgent({ cwd, label, agentType }, ctx);
			persistState(pi);
		},
	});

	pi.registerCommand("agents-steer", {
		description: "Send a steer message to an agent. Usage: /agents-steer <id|label> <message>",
		handler: async (args, ctx) => {
			const spaceIdx = args.indexOf(" ");
			if (spaceIdx === -1) {
				ctx.ui.notify("Usage: /agents-steer <id|label> <message>", "warning");
				return;
			}
			const idOrLabel = args.slice(0, spaceIdx).trim();
			const message = args.slice(spaceIdx + 1).trim();
			const agent = resolveAgentId(idOrLabel);
			if (!agent) {
				ctx.ui.notify(`No agent found: ${idOrLabel}`, "error");
				return;
			}
			writeToFifo(agent, { type: "steer", message });
			ctx.ui.notify(`Steered ${agent.label}`, "info");
		},
	});

	pi.registerCommand("agents-status", {
		description: "Print status of all managed agents to chat",
		handler: async (_args, ctx) => {
			printStatusToChat(ctx);
		},
	});

	// ── Shortcut ──────────────────────────────────────────────────────

	pi.registerShortcut("ctrl+shift+m", {
		description: "Open/close Agent Manager dashboard",
		handler: async (ctx) => {
			await openDashboard(ctx, pi);
		},
	});

	// ── Tool: agent_manager ───────────────────────────────────────────

	const AgentManagerParams = Type.Object({
		action: StringEnum(["spawn", "steer", "abort", "status", "kill", "follow_up"] as const),
		agentId: Type.Optional(Type.String({ description: "Agent id or label" })),
		cwd: Type.Optional(Type.String({ description: "Repo path (for spawn)" })),
		sessionFile: Type.Optional(Type.String({ description: "Session file path (for spawn)" })),
		message: Type.Optional(Type.String({ description: "Message for steer/follow_up" })),
	});

	pi.registerTool({
		name: "agent_manager",
		label: "Agent Manager",
		description: [
			"Manage headless RPC agents running in other repos.",
			"Actions: spawn (start new agent), steer (interrupt + redirect), abort (cancel run),",
			"follow_up (queue message for after current run), status (show all agents), kill (stop + remove).",
		].join(" "),
		parameters: AgentManagerParams,

		async execute(_toolCallId, params, _signal, _onUpdate, ctx) {
			const err = (msg: string) => ({
				content: [{ type: "text" as const, text: msg }],
				details: {} as unknown,
				isError: true as const,
			});
			const ok = (msg: string) => ({
				content: [{ type: "text" as const, text: msg }],
				details: {} as unknown,
			});

			switch (params.action) {
				case "spawn": {
					if (!params.cwd) return err("Error: cwd is required for spawn");
					const id = await spawnAgent({ cwd: params.cwd, sessionFile: params.sessionFile }, ctx);
					persistState(pi);
					return ok(`Spawned agent ${id} in ${params.cwd}`);
				}

				case "status": {
					const agents = getAgentsList();
					if (agents.length === 0) return ok("No agents running.");
					const lines = agents.map((a) => {
						const done = a.workflowSteps?.filter((s) => s.done).length ?? 0;
						const total = a.workflowSteps?.length ?? "?";
						const issue = a.workflowIssue ? ` | #${a.workflowIssue.number}` : "";
						return `${statusIcon(a.status)} ${a.label} [${a.status}${issue}] workflow ${done}/${total}${a.lastToolCall ? ` | ${a.lastToolCall}` : ""}`;
					});
					return ok(lines.join("\n"));
				}

				case "steer":
				case "follow_up": {
					if (!params.agentId || !params.message) return err("Error: agentId and message are required");
					const agent = resolveAgentId(params.agentId);
					if (!agent) return err(`Error: agent not found: ${params.agentId}`);
					const cmdType = params.action === "follow_up" ? "follow_up" : "steer";
					writeToFifo(agent, { type: cmdType, message: params.message });
					return ok(`Sent ${cmdType} to ${agent.label}`);
				}

				case "abort": {
					if (!params.agentId) return err("Error: agentId is required for abort");
					const agent = resolveAgentId(params.agentId);
					if (!agent) return err(`Error: agent not found: ${params.agentId}`);
					writeToFifo(agent, { type: "abort" });
					return ok(`Aborted ${agent.label}`);
				}

				case "kill": {
					if (!params.agentId) return err("Error: agentId is required for kill");
					const agent = resolveAgentId(params.agentId);
					if (!agent) return err(`Error: agent not found: ${params.agentId}`);
					killAgent(agent.id);
					return ok(`Killed and removed ${agent.label}`);
				}

				default:
					return err(`Unknown action: ${String(params.action)}`);
			}
		},

		renderCall(args, theme) {
			let text =
				theme.fg("toolTitle", theme.bold("agent_manager ")) +
				theme.fg("muted", args.action);
			if (args.agentId) text += " " + theme.fg("accent", args.agentId);
			if (args.cwd) text += " " + theme.fg("dim", args.cwd);
			return new Text(text, 0, 0);
		},

		renderResult(result, _opts, theme) {
			const msg = result.content[0];
			const text = msg?.type === "text" ? msg.text : "(no output)";
			if ((result as { isError?: boolean }).isError) return new Text(theme.fg("error", `✗ ${text}`), 0, 0);
			return new Text(theme.fg("muted", text), 0, 0);
		},
	});
}
