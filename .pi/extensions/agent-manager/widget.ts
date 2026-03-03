/**
 * Agent Manager — compact always-visible widget.
 *
 * Renders a multi-line widget above the editor showing all agents at a glance.
 * Called via `ctx.ui.setWidget("agent-manager", ...)`.
 */
import { Text, truncateToWidth } from "@mariozechner/pi-tui";
import type { Theme } from "@mariozechner/pi-coding-agent";
import {
	type ManagedAgent,
	computeStatusSummary,
	formatProgress,
	statusIcon,
} from "./agent-state.js";

const WIDGET_PROGRESS_BAR_WIDTH = 15;

/**
 * Build the widget render function for `ctx.ui.setWidget`.
 * Returns undefined (hide widget) when no agents are registered.
 */
export function buildWidgetRenderer(
	agents: ManagedAgent[],
): ((_tui: unknown, theme: Theme) => Text) | undefined {
	if (agents.length === 0) return undefined;

	return (_tui: unknown, theme: Theme) => {
		const lines: string[] = [];

		// ── Header ──
		const summary = computeStatusSummary(agents);
		const parts: string[] = [];
		if (summary.running > 0) parts.push(theme.fg("success", `${summary.running} running`));
		if (summary.idle > 0) parts.push(theme.fg("muted", `${summary.idle} idle`));
		if (summary.blocked > 0) parts.push(theme.fg("warning", `${summary.blocked} blocked`));
		if (summary.error > 0) parts.push(theme.fg("error", `${summary.error} errored`));

		const summaryStr = parts.length > 0 ? ` [${parts.join(" · ")}]` : "";
		const shortcut = theme.fg("dim", "Ctrl+Shift+M to open");
		lines.push(
			theme.fg("accent", theme.bold(" Agents")) +
				theme.fg("muted", summaryStr) +
				"  " +
				shortcut,
		);

		// ── Agent rows ──
		for (const agent of agents) {
			lines.push(renderAgentRow(agent, theme));
		}

		const text = lines.join("\n");
		return new Text(text, 0, 0);
	};
}

function renderAgentRow(agent: ManagedAgent, theme: Theme): string {
	const icon = coloredStatusIcon(agent.status, theme);
	const label = theme.fg("text", agent.label.slice(0, 18).padEnd(18));

	// Issue/task description
	let taskStr = "";
	if (agent.workflowIssue) {
		const titlePreview = agent.workflowIssue.title.slice(0, 25);
		taskStr =
			theme.fg("accent", `#${agent.workflowIssue.number}`) +
			" " +
			theme.fg("dim", titlePreview + (agent.workflowIssue.title.length > 25 ? "…" : ""));
	} else if (agent.lastText) {
		taskStr = theme.fg("dim", agent.lastText.slice(0, 28));
	}
	const task = taskStr.padEnd(35);

	// Workflow progress bar
	const bar = formatProgress(agent.workflowSteps, WIDGET_PROGRESS_BAR_WIDTH);
	const doneCount = agent.workflowSteps?.filter((s) => s.done).length ?? 0;
	const totalCount = agent.workflowSteps?.length ?? 16;
	const progressBar =
		theme.fg("success", bar.replace(/░/g, "")) +
		theme.fg("dim", bar.replace(/█/g, ""));
	const stepsStr = theme.fg("muted", `${doneCount}/${totalCount}`);

	// Last action
	const actionStr = agent.lastToolCall
		? theme.fg("dim", agent.lastToolCall.slice(0, 30))
		: agent.status === "running"
			? theme.fg("dim", "running…")
			: "";

	return `  ${icon} ${label} ${task} ${progressBar} ${stepsStr}  ${actionStr}`;
}

function coloredStatusIcon(
	status: ManagedAgent["status"],
	theme: Theme,
): string {
	const icon = statusIcon(status);
	switch (status) {
		case "running":
			return theme.fg("success", icon);
		case "idle":
			return theme.fg("muted", icon);
		case "blocked":
			return theme.fg("warning", icon);
		case "error":
			return theme.fg("error", icon);
		case "starting":
			return theme.fg("dim", icon);
		case "done":
			return theme.fg("success", icon);
	}
}

/**
 * Truncate rendered widget lines to a given terminal width.
 */
export function truncateWidgetLines(lines: string[], width: number): string[] {
	return lines.map((l) => truncateToWidth(l, width));
}
