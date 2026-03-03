/**
 * Agent Manager — full-screen TUI dashboard component.
 *
 * Opened via `/agents` command or Ctrl+Shift+M.
 * Shows a tabbed view of all managed agents with send-message input.
 */
import { matchesKey, Text, truncateToWidth } from "@mariozechner/pi-tui";
import type { Theme } from "@mariozechner/pi-coding-agent";
import {
	type ManagedAgent,
	computeStatusSummary,
	formatProgress,
	statusIcon,
} from "./agent-state.js";

export type DashboardAction =
	| { action: "close" }
	| { action: "send"; agentId: string; message: string; as: "prompt" | "steer" | "followUp" }
	| { action: "abort"; agentId: string }
	| { action: "kill"; agentId: string }
	| { action: "relaunch"; agentId: string }
	| { action: "spawn" };

/**
 * Full-screen dashboard component for ctx.ui.custom().
 *
 * Usage:
 *   const action = await ctx.ui.custom<DashboardAction>(
 *     (tui, theme, _kb, done) => new AgentDashboard(agents, theme, done)
 *   );
 */
export class AgentDashboard {
	private agents: ManagedAgent[];
	private theme: Theme;
	private done: (result: DashboardAction) => void;
	private selectedTab = 0;
	private inputBuffer = "";
	private inputMode = false;
	private inputPrompt = "Follow-up";
	private inputAs: "prompt" | "steer" | "followUp" = "followUp";
	private cachedWidth?: number;
	private cachedLines?: string[];

	constructor(
		agents: ManagedAgent[],
		theme: Theme,
		done: (result: DashboardAction) => void,
	) {
		this.agents = agents;
		this.theme = theme;
		this.done = done;
		this.selectedTab = 0;
	}

	/** Called by pi TUI to render. Returns array of lines. */
	render(width: number): string[] {
		if (this.cachedLines && this.cachedWidth === width) return this.cachedLines;

		const th = this.theme;
		const lines: string[] = [];
		const bar = (char: string) => th.fg("borderMuted", char.repeat(width));

		// ── Title bar ────────────────────────────────────────────────────
		lines.push(bar("─"));
		const title = th.fg("accent", th.bold(" Agent Manager "));
		const summary = this.renderSummary(width);
		const help = th.fg("dim", "↑↓ switch  Esc close");
		lines.push(truncateToWidth(` ${title}  ${summary}  ${help}`, width));
		lines.push(bar("─"));

		// ── Tab bar ──────────────────────────────────────────────────────
		if (this.agents.length === 0) {
			lines.push(th.fg("muted", "  No agents. Press N to spawn one."));
		} else {
			lines.push(this.renderTabBar(width));
			lines.push(bar("─"));
			lines.push(...this.renderAgentPane(width));
		}

		// ── Input area ───────────────────────────────────────────────────
		lines.push(bar("─"));
		if (this.inputMode) {
			lines.push(
				truncateToWidth(
					th.fg("muted", `  ${this.inputPrompt} > `) + th.fg("text", this.inputBuffer) + th.fg("accent", "▌"),
					width,
				),
			);
			lines.push(
				truncateToWidth(
					th.fg("dim", "  Enter send  ·  Esc cancel"),
					width,
				),
			);
		} else {
			lines.push(
				truncateToWidth(
					th.fg("dim", "  Enter follow-up  ·  S steer  ·  A abort  ·  K kill  ·  N spawn  ·  R relaunch  ·  Esc close"),
					width,
				),
			);
		}
		lines.push(bar("─"));

		this.cachedWidth = width;
		this.cachedLines = lines;
		return lines;
	}

	/** Called by pi TUI on keyboard input. */
	handleInput(data: string): void {
		this.invalidate();

		if (this.inputMode) {
			this.handleInputModeKey(data);
			return;
		}

		// Tab navigation
		if (matchesKey(data, "tab") || data === "]") {
			this.selectedTab = (this.selectedTab + 1) % Math.max(1, this.agents.length);
			return;
		}
		if ((matchesKey(data, "tab") && data.includes("shift")) || data === "[") {
			this.selectedTab =
				(this.selectedTab - 1 + Math.max(1, this.agents.length)) %
				Math.max(1, this.agents.length);
			return;
		}
		if (matchesKey(data, "up") || data === "k") {
			this.selectedTab = Math.max(0, this.selectedTab - 1);
			return;
		}
		if (matchesKey(data, "down") || data === "j") {
			this.selectedTab = Math.min(this.agents.length - 1, this.selectedTab + 1);
			return;
		}

		// Agent actions
		const agent = this.agents[this.selectedTab];

		if (matchesKey(data, "escape") || matchesKey(data, "ctrl+c")) {
			this.done({ action: "close" });
			return;
		}

		if (matchesKey(data, "return") && agent) {
			// Enter follow-up message
			this.inputMode = true;
			this.inputBuffer = "";
			this.inputPrompt = "Follow-up";
			this.inputAs = "followUp";
			return;
		}

		if ((data === "s" || data === "S") && agent) {
			this.inputMode = true;
			this.inputBuffer = "";
			this.inputPrompt = "Steer";
			this.inputAs = "steer";
			return;
		}

		if ((data === "a" || data === "A") && agent) {
			this.done({ action: "abort", agentId: agent.id });
			return;
		}

		if ((data === "k" || data === "K") && agent) {
			this.done({ action: "kill", agentId: agent.id });
			return;
		}

		if (data === "n" || data === "N") {
			this.done({ action: "spawn" });
			return;
		}

		if ((data === "r" || data === "R") && agent) {
			this.done({ action: "relaunch", agentId: agent.id });
			return;
		}
	}

	// ── Private ────────────────────────────────────────────────────────────

	private handleInputModeKey(data: string): void {
		if (matchesKey(data, "escape") || matchesKey(data, "ctrl+c")) {
			this.inputMode = false;
			this.inputBuffer = "";
			return;
		}

		if (matchesKey(data, "return")) {
			const msg = this.inputBuffer.trim();
			if (msg && this.agents[this.selectedTab]) {
				this.done({
					action: "send",
					agentId: this.agents[this.selectedTab].id,
					message: msg,
					as: this.inputAs,
				});
			} else {
				this.inputMode = false;
				this.inputBuffer = "";
			}
			return;
		}

		if (matchesKey(data, "backspace")) {
			this.inputBuffer = this.inputBuffer.slice(0, -1);
			return;
		}

		// Printable character
		if (data.length === 1 && data.charCodeAt(0) >= 32) {
			this.inputBuffer += data;
		}
	}

	private renderSummary(width: number): string {
		const th = this.theme;
		const s = computeStatusSummary(this.agents);
		const parts: string[] = [];
		if (s.running > 0) parts.push(th.fg("success", `${s.running} running`));
		if (s.idle > 0) parts.push(th.fg("muted", `${s.idle} idle`));
		if (s.blocked > 0) parts.push(th.fg("warning", `${s.blocked} blocked`));
		if (s.error > 0) parts.push(th.fg("error", `${s.error} errored`));
		void width;
		return parts.join(" · ");
	}

	private renderTabBar(width: number): string {
		const th = this.theme;
		let tabs = "";
		for (let i = 0; i < this.agents.length; i++) {
			const a = this.agents[i];
			const isSel = i === this.selectedTab;
			const icon = statusIcon(a.status);
			const label = a.label.slice(0, 16);
			if (isSel) {
				tabs += th.fg("accent", th.bold(` [${icon} ${label}] `));
			} else {
				tabs += th.fg("muted", ` ${icon} ${label}  `);
			}
		}
		tabs += th.fg("dim", " [+ N spawn]");
		return truncateToWidth(tabs, width);
	}

	private renderAgentPane(width: number): string[] {
		const th = this.theme;
		const agent = this.agents[this.selectedTab];
		if (!agent) return [th.fg("muted", "  Select an agent")];

		const lines: string[] = [];

		// Status line
		const statusColor = agent.status === "running" ? "success" : agent.status === "error" ? "error" : "muted";
		const costStr = `$${agent.usage.cost.toFixed(2)}`;
		const tokStr = formatTokenCount(
			agent.usage.input + agent.usage.output + agent.usage.cacheRead + agent.usage.cacheWrite,
		);
		lines.push(
			truncateToWidth(
				` ${th.fg("text", th.bold(agent.label))}` +
					`  ${th.fg(statusColor, agent.status)}` +
					`  ${th.fg("dim", `${costStr}  ${tokStr} tok`)}` +
					(agent.model ? `  ${th.fg("dim", agent.model)}` : ""),
				width,
			),
		);

		// Issue + workflow
		if (agent.workflowIssue) {
			lines.push(
				truncateToWidth(
					` Issue ${th.fg("accent", `#${agent.workflowIssue.number}`)}: ${th.fg("muted", agent.workflowIssue.title)}`,
					width,
				),
			);
		}
		if (agent.workflowSteps && agent.workflowSteps.length > 0) {
			const done = agent.workflowSteps.filter((s) => s.done).length;
			const total = agent.workflowSteps.length;
			const barWidth = Math.min(30, width - 25);
			const bar = formatProgress(agent.workflowSteps, barWidth);
			const currentStep = agent.workflowSteps.find((s) => !s.done);
			lines.push(
				truncateToWidth(
					` Workflow: ${th.fg("success", bar.replace(/░/g, ""))}${th.fg("dim", bar.replace(/█/g, ""))}` +
						` ${done}/${total}` +
						(currentStep ? `  → Step ${currentStep.id}${currentStep.label ? `: ${currentStep.label}` : ""}` : "  ✓"),
					width,
				),
			);
		}

		lines.push("");

		// Last action
		if (agent.lastToolCall) {
			lines.push(th.fg("muted", " Last action:"));
			lines.push(truncateToWidth(`   ${th.fg("dim", agent.lastToolCall)}`, width));
			lines.push("");
		}

		// Last output
		if (agent.lastText) {
			lines.push(th.fg("muted", " Last output:"));
			const preview = agent.lastText.split("\n").slice(0, 4).join("\n");
			for (const l of preview.split("\n")) {
				lines.push(truncateToWidth(`   ${th.fg("toolOutput", l)}`, width));
			}
			lines.push("");
		}

		// Alerts
		const unread = agent.alerts.filter((a) => !a.read);
		if (unread.length > 0) {
			lines.push(th.fg("warning", ` ⚠ ${unread.length} unread alert(s):`));
			for (const alert of unread.slice(0, 3)) {
				lines.push(truncateToWidth(`   ${th.fg("warning", alert.message)}`, width));
			}
			lines.push("");
		}

		return lines;
	}

	invalidate(): void {
		this.cachedWidth = undefined;
		this.cachedLines = undefined;
	}
}

// ─── Helpers ──────────────────────────────────────────────────────────────

function formatTokenCount(n: number): string {
	if (n < 1_000) return String(n);
	if (n < 10_000) return `${(n / 1_000).toFixed(1)}k`;
	if (n < 1_000_000) return `${Math.round(n / 1_000)}k`;
	return `${(n / 1_000_000).toFixed(1)}M`;
}
