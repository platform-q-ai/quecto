/**
 * Heartbeat Extension
 *
 * Fires a configurable `pi.sendUserMessage()` follow-up on a recurring interval
 * (default: every 5 minutes), nudging the controller LLM to check in on managed
 * agents and take corrective action for any stalled, blocked, or errored ones.
 *
 * This extension is intentionally standalone — it pairs well with agent-manager
 * but has no dependency on it. It can be used for any recurring triage or
 * check-in workflow.
 *
 * Commands:
 *   /heartbeat-on        — enable heartbeat (starts timer)
 *   /heartbeat-off       — disable heartbeat (clears timer)
 *   /heartbeat-config    — configure interval (minutes) and custom prompt
 *   /heartbeat-now       — fire a heartbeat tick immediately
 *
 * Shortcut:
 *   Ctrl+Shift+H         — toggle heartbeat on/off
 *
 * Footer status:
 *   ♥ heartbeat  next in Xm Ys
 */

import type { ExtensionAPI, ExtensionContext } from "@mariozechner/pi-coding-agent";

// ─── Public constants (exported for tests) ────────────────────────────────

export const DEFAULT_INTERVAL_MS = 300_000; // 5 minutes

export const DEFAULT_HEARTBEAT_PROMPT = `[Heartbeat] Check the status of all managed agents via agent_manager. For any agent \
that is blocked, errored, or idle with incomplete workflow steps, take appropriate \
action: steer it past the blockage, restart it if crashed, or flag it for human \
review if intervention is needed. If all agents are healthy and progressing, respond \
with a single line summary and stand by for the next heartbeat.`;

// ─── Pure helpers (exported for tests) ────────────────────────────────────

/**
 * Format milliseconds remaining into a human-readable countdown string.
 * Examples: "42s", "3m 42s", "0s"
 */
export function formatNextTick(remainingMs: number): string {
	const totalSeconds = Math.max(0, Math.floor(remainingMs / 1_000));
	const minutes = Math.floor(totalSeconds / 60);
	const seconds = totalSeconds % 60;
	if (minutes === 0) return `${seconds}s`;
	return `${minutes}m ${seconds}s`;
}

/**
 * Determine whether the current tick should be skipped.
 */
export function shouldSkipTick(opts: { hasPendingMessages: boolean }): boolean {
	return opts.hasPendingMessages;
}

/**
 * Build the heartbeat prompt — returns custom if set, otherwise default.
 */
export function buildHeartbeatPrompt(custom: string | undefined): string {
	return custom && custom.trim() ? custom.trim() : DEFAULT_HEARTBEAT_PROMPT;
}

// ─── State ────────────────────────────────────────────────────────────────

interface HeartbeatState {
	enabled: boolean;
	intervalMs: number;
	prompt: string | undefined;
	lastFiredAt?: number;
}

function defaultState(): HeartbeatState {
	return {
		enabled: false,
		intervalMs: DEFAULT_INTERVAL_MS,
		prompt: undefined,
	};
}

// ─── Extension entry point ────────────────────────────────────────────────

export default function (pi: ExtensionAPI) {
	let state: HeartbeatState = defaultState();
	let timerId: ReturnType<typeof setInterval> | null = null;
	let tickStartedAt = 0; // timestamp when last interval started
	let statusUpdateId: ReturnType<typeof setInterval> | null = null;

	// ── State reconstruction ──────────────────────────────────────────

	function reconstructState(ctx: ExtensionContext): void {
		state = defaultState();
		const entries = ctx.sessionManager.getBranch();
		for (const entry of entries) {
			if (entry.type === "custom" && entry.customType === "heartbeat-state") {
				const d = (entry as { data?: HeartbeatState }).data;
				if (d && typeof d.enabled === "boolean") {
					state = {
						enabled: d.enabled,
						intervalMs: typeof d.intervalMs === "number" ? d.intervalMs : DEFAULT_INTERVAL_MS,
						prompt: typeof d.prompt === "string" ? d.prompt : undefined,
						lastFiredAt: typeof d.lastFiredAt === "number" ? d.lastFiredAt : undefined,
					};
				}
			}
		}
	}

	function persistState(): void {
		pi.appendEntry("heartbeat-state", { ...state });
	}

	// ── Timer management ──────────────────────────────────────────────

	function startTimer(ctx: ExtensionContext): void {
		stopTimer(ctx);
		tickStartedAt = Date.now();

		timerId = setInterval(() => {
			fireTick(ctx);
		}, state.intervalMs);

		startStatusUpdater(ctx);
		updateFooterStatus(ctx);
	}

	function stopTimer(ctx: ExtensionContext): void {
		if (timerId !== null) {
			clearInterval(timerId);
			timerId = null;
		}
		stopStatusUpdater();
		clearFooterStatus(ctx);
	}

	function startStatusUpdater(ctx: ExtensionContext): void {
		stopStatusUpdater();
		statusUpdateId = setInterval(() => updateFooterStatus(ctx), 1_000);
	}

	function stopStatusUpdater(): void {
		if (statusUpdateId !== null) {
			clearInterval(statusUpdateId);
			statusUpdateId = null;
		}
	}

	// ── Tick logic ────────────────────────────────────────────────────

	function fireTick(ctx: ExtensionContext): void {
		if (shouldSkipTick({ hasPendingMessages: ctx.hasPendingMessages() })) {
			return; // Silently skip — messages already queued
		}

		const prompt = buildHeartbeatPrompt(state.prompt);
		pi.sendUserMessage(prompt, { deliverAs: "followUp" });

		state = { ...state, lastFiredAt: Date.now() };
		tickStartedAt = Date.now(); // reset countdown
		persistState();
		updateFooterStatus(ctx);
	}

	// ── Footer status ─────────────────────────────────────────────────

	function updateFooterStatus(ctx: ExtensionContext): void {
		if (!state.enabled || timerId === null) {
			clearFooterStatus(ctx);
			return;
		}
		const elapsed = Date.now() - tickStartedAt;
		const remaining = Math.max(0, state.intervalMs - elapsed);
		const countdown = formatNextTick(remaining);
		ctx.ui.setStatus("heartbeat", `♥ heartbeat  next in ${countdown}`);
	}

	function clearFooterStatus(ctx: ExtensionContext): void {
		ctx.ui.setStatus("heartbeat", undefined);
	}

	// ── Session lifecycle ─────────────────────────────────────────────

	pi.on("session_start", async (_event, ctx) => {
		reconstructState(ctx);
		if (state.enabled) {
			startTimer(ctx);
		}
	});

	pi.on("session_switch", async (_event, ctx) => {
		stopTimer(ctx);
		reconstructState(ctx);
		if (state.enabled) {
			startTimer(ctx);
		}
	});

	pi.on("session_shutdown", async (_event, ctx) => {
		stopTimer(ctx);
	});

	// ── Commands ──────────────────────────────────────────────────────

	pi.registerCommand("heartbeat-on", {
		description: "Enable heartbeat timer (fires follow-up every 5 minutes by default)",
		handler: async (_args, ctx) => {
			if (state.enabled) {
				ctx.ui.notify("Heartbeat is already enabled", "info");
				return;
			}
			state = { ...state, enabled: true };
			persistState();
			startTimer(ctx);
			const intervalMin = Math.round(state.intervalMs / 60_000);
			ctx.ui.notify(`♥ Heartbeat enabled — fires every ${intervalMin}m`, "info");
		},
	});

	pi.registerCommand("heartbeat-off", {
		description: "Disable heartbeat timer",
		handler: async (_args, ctx) => {
			if (!state.enabled) {
				ctx.ui.notify("Heartbeat is already disabled", "info");
				return;
			}
			state = { ...state, enabled: false };
			persistState();
			stopTimer(ctx);
			ctx.ui.notify("Heartbeat disabled", "info");
		},
	});

	pi.registerCommand("heartbeat-config", {
		description: "Configure heartbeat interval and prompt",
		handler: async (_args, ctx) => {
			if (!ctx.hasUI) {
				ctx.ui.notify(
					`Heartbeat: ${state.enabled ? "on" : "off"}, interval ${state.intervalMs}ms`,
					"info",
				);
				return;
			}

			// Ask for interval
			const intervalStr = await ctx.ui.input(
				"Interval (minutes):",
				String(state.intervalMs / 60_000),
			);
			if (intervalStr === undefined) return; // cancelled

			const intervalMin = Number(intervalStr.trim());
			if (isNaN(intervalMin) || intervalMin < 1) {
				ctx.ui.notify("Invalid interval. Must be ≥1 minute.", "error");
				return;
			}

			// Ask for custom prompt
			const promptInput = await ctx.ui.input(
				"Custom prompt (leave empty for default):",
				state.prompt ?? "",
			);
			if (promptInput === undefined) return; // cancelled

			const wasEnabled = state.enabled;
			state = {
				...state,
				intervalMs: intervalMin * 60_000,
				prompt: promptInput.trim() || undefined,
			};
			persistState();

			// Restart timer if enabled
			if (wasEnabled) {
				startTimer(ctx);
			}

			ctx.ui.notify(
				`Heartbeat configured: ${intervalMin}m interval` +
					(state.prompt ? ", custom prompt" : ", default prompt"),
				"info",
			);
		},
	});

	pi.registerCommand("heartbeat-now", {
		description: "Fire a heartbeat tick immediately",
		handler: async (_args, ctx) => {
			fireTick(ctx);
			ctx.ui.notify("Heartbeat fired", "info");
		},
	});

	// ── Shortcut: Ctrl+Shift+H toggle ────────────────────────────────

	pi.registerShortcut("ctrl+shift+h", {
		description: "Toggle heartbeat on/off",
		handler: async (ctx) => {
			if (state.enabled) {
				state = { ...state, enabled: false };
				persistState();
				stopTimer(ctx);
				ctx.ui.notify("Heartbeat disabled", "info");
			} else {
				state = { ...state, enabled: true };
				persistState();
				startTimer(ctx);
				const intervalMin = Math.round(state.intervalMs / 60_000);
				ctx.ui.notify(`♥ Heartbeat enabled — fires every ${intervalMin}m`, "info");
			}
		},
	});
}
