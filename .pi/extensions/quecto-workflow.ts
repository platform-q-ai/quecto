/**
 * Quecto Workflow Extension - Enforces the BDD/TDD Red-Green-Refactor development workflow
 * from AGENTS.md as an interactive todo checklist in Pi.
 *
 * The workflow has 15 steps:
 *  1. Update Scenarios / Add new features as necessary
 *  2. Write/update unit tests
 *  3. Ensure new/modified tests fail (RED)
 *  4. Implement code (GREEN)
 *  5. Refactor (performance, security, clean architecture)
 *  6. Ensure tests still pass (GREEN)
 *  7. Commit
 *  8. Push
 *  9. Create PR
 * 10. Despatch Architecture, Security, Performance Reviewers
 * 11. Fix all valid concerns raised in review comments
 * 12. Push changes to remote
 * 13. Reply to comments and mark resolved
 * 14. Merge
 * 15. Move to local master and pull
 *
 * Features:
 * - `/workflow` command opens an interactive checklist UI
 * - `workflow` tool lets the LLM check/uncheck/reset/query steps
 * - Widget above editor shows current progress at a glance
 * - Blocks git commit if RED/GREEN/REFACTOR steps aren't done
 * - Injects workflow awareness into the system prompt
 * - State persists across session restarts via tool result details
 */

import { StringEnum } from "@mariozechner/pi-ai";
import type { ExtensionAPI, ExtensionContext, Theme } from "@mariozechner/pi-coding-agent";
import { isToolCallEventType } from "@mariozechner/pi-coding-agent";
import { matchesKey, Text, truncateToWidth } from "@mariozechner/pi-tui";
import { Type } from "@sinclair/typebox";

// ─── Workflow definition ───────────────────────────────────────────────

interface WorkflowStep {
	id: number;
	label: string;
	phase: "red" | "green" | "refactor" | "ci" | "review";
	done: boolean;
}

const WORKFLOW_TEMPLATE: Omit<WorkflowStep, "done">[] = [
	{ id: 1, label: "Update Scenarios / Add new features", phase: "red" },
	{ id: 2, label: "Write/update unit tests", phase: "red" },
	{ id: 3, label: "Ensure new/modified tests FAIL (RED)", phase: "red" },
	{ id: 4, label: "Implement code (GREEN)", phase: "green" },
	{ id: 5, label: "Refactor (perf, security, clean arch)", phase: "refactor" },
	{ id: 6, label: "Ensure tests still pass (GREEN)", phase: "green" },
	{ id: 7, label: "Commit", phase: "ci" },
	{ id: 8, label: "Push", phase: "ci" },
	{ id: 9, label: "Create PR", phase: "ci" },
	{ id: 10, label: "Despatch reviewers (Arch, Security, Perf)", phase: "review" },
	{ id: 11, label: "Fix all valid review concerns", phase: "review" },
	{ id: 12, label: "Push changes to remote", phase: "review" },
	{ id: 13, label: "Reply to comments and mark resolved", phase: "review" },
	{ id: 14, label: "Merge", phase: "ci" },
	{ id: 15, label: "Move to local master and pull", phase: "ci" },
];

function freshSteps(): WorkflowStep[] {
	return WORKFLOW_TEMPLATE.map((s) => ({ ...s, done: false }));
}

// ─── Tool details shape (for state persistence) ───────────────────────

interface WorkflowDetails {
	action: "status" | "check" | "uncheck" | "reset" | "skip";
	steps: WorkflowStep[];
	error?: string;
}

// ─── Phase colors ─────────────────────────────────────────────────────

function phaseColor(phase: string, theme: Theme): (t: string) => string {
	switch (phase) {
		case "red":
			return (t) => theme.fg("error", t);
		case "green":
			return (t) => theme.fg("success", t);
		case "refactor":
			return (t) => theme.fg("warning", t);
		case "ci":
			return (t) => theme.fg("accent", t);
		case "review":
			return (t) => theme.fg("muted", t);
		default:
			return (t) => t;
	}
}

function phaseLabel(phase: string): string {
	switch (phase) {
		case "red":
			return "RED";
		case "green":
			return "GREEN";
		case "refactor":
			return "REFACTOR";
		case "ci":
			return "CI/CD";
		case "review":
			return "REVIEW";
		default:
			return phase;
	}
}

// ─── Interactive checklist component ──────────────────────────────────

class WorkflowChecklist {
	private steps: WorkflowStep[];
	private theme: Theme;
	private onClose: (steps: WorkflowStep[]) => void;
	private selected: number = 0;
	private cachedWidth?: number;
	private cachedLines?: string[];

	constructor(steps: WorkflowStep[], theme: Theme, onClose: (steps: WorkflowStep[]) => void) {
		this.steps = steps.map((s) => ({ ...s })); // deep copy
		this.theme = theme;
		this.onClose = onClose;
	}

	handleInput(data: string): void {
		if (matchesKey(data, "escape") || matchesKey(data, "ctrl+c")) {
			this.onClose(this.steps);
			return;
		}
		if (matchesKey(data, "up") || data === "k") {
			this.selected = Math.max(0, this.selected - 1);
			this.invalidate();
			return;
		}
		if (matchesKey(data, "down") || data === "j") {
			this.selected = Math.min(this.steps.length - 1, this.selected + 1);
			this.invalidate();
			return;
		}
		if (matchesKey(data, "return") || data === " " || data === "x") {
			this.steps[this.selected].done = !this.steps[this.selected].done;
			this.invalidate();
			return;
		}
		if (data === "r" || data === "R") {
			this.steps.forEach((s) => (s.done = false));
			this.invalidate();
			return;
		}
	}

	render(width: number): string[] {
		if (this.cachedLines && this.cachedWidth === width) {
			return this.cachedLines;
		}

		const th = this.theme;
		const lines: string[] = [];

		lines.push("");
		const title = th.fg("accent", th.bold(" Quecto Dev Workflow "));
		const bar = th.fg("borderMuted", "─".repeat(3)) + title + th.fg("borderMuted", "─".repeat(Math.max(0, width - 26)));
		lines.push(truncateToWidth(bar, width));
		lines.push(truncateToWidth(`  ${th.fg("dim", "BDD/TDD Red → Green → Refactor")}`, width));
		lines.push("");

		const done = this.steps.filter((s) => s.done).length;
		const total = this.steps.length;
		const pct = Math.round((done / total) * 100);
		const barLen = Math.min(30, width - 20);
		const filled = Math.round((done / total) * barLen);
		const progressBar =
			th.fg("success", "█".repeat(filled)) + th.fg("dim", "░".repeat(barLen - filled));
		lines.push(truncateToWidth(`  ${progressBar} ${th.fg("muted", `${done}/${total} (${pct}%)`)}`, width));
		lines.push("");

		let lastPhase = "";
		for (let i = 0; i < this.steps.length; i++) {
			const step = this.steps[i];

			// Phase header
			if (step.phase !== lastPhase) {
				lastPhase = step.phase;
				const colorFn = phaseColor(step.phase, th);
				lines.push(truncateToWidth(`  ${colorFn(th.bold(phaseLabel(step.phase)))}`, width));
			}

			const isSel = i === this.selected;
			const check = step.done ? th.fg("success", "✓") : th.fg("dim", "○");
			const num = th.fg("accent", `${step.id.toString().padStart(2)}.`);
			const text = step.done ? th.fg("dim", step.label) : th.fg("text", step.label);
			const pointer = isSel ? th.fg("accent", "▸ ") : "  ";

			lines.push(truncateToWidth(`  ${pointer}${check} ${num} ${text}`, width));
		}

		lines.push("");
		lines.push(
			truncateToWidth(
				`  ${th.fg("dim", "↑↓ navigate  ·  Enter/Space toggle  ·  R reset  ·  Esc close")}`,
				width,
			),
		);
		lines.push("");

		this.cachedWidth = width;
		this.cachedLines = lines;
		return lines;
	}

	invalidate(): void {
		this.cachedWidth = undefined;
		this.cachedLines = undefined;
	}
}

// ─── Extension entry point ────────────────────────────────────────────

export default function (pi: ExtensionAPI) {
	let steps: WorkflowStep[] = freshSteps();

	// ── State reconstruction ──────────────────────────────────────────

	const reconstructState = (ctx: ExtensionContext) => {
		steps = freshSteps();

		const entries = ctx.sessionManager.getBranch();
		let lastToolResultIdx = -1;
		let lastAppendIdx = -1;

		// First pass: find tool results
		for (let i = 0; i < entries.length; i++) {
			const entry = entries[i];
			if (entry.type === "message") {
				const msg = entry.message;
				if (msg.role === "toolResult" && msg.toolName === "workflow") {
					const details = msg.details as WorkflowDetails | undefined;
					if (details?.steps) {
						steps = details.steps.map((s) => ({ ...s }));
						lastToolResultIdx = i;
					}
				}
			}
			if (entry.type === "custom" && entry.customType === "workflow-state") {
				lastAppendIdx = i;
			}
		}

		// If manual state (from /workflow command) is newer than last tool result, use it
		if (lastAppendIdx > lastToolResultIdx) {
			const entry = entries[lastAppendIdx];
			if (entry.type === "custom" && entry.data?.steps) {
				steps = (entry.data.steps as WorkflowStep[]).map((s) => ({ ...s }));
			}
		}

		updateWidget(ctx);
	};

	pi.on("session_start", async (_event, ctx) => reconstructState(ctx));
	pi.on("session_switch", async (_event, ctx) => reconstructState(ctx));
	pi.on("session_fork", async (_event, ctx) => reconstructState(ctx));
	pi.on("session_tree", async (_event, ctx) => reconstructState(ctx));

	// ── Widget: always-visible progress ───────────────────────────────

	const updateWidget = (ctx: ExtensionContext) => {
		const done = steps.filter((s) => s.done).length;
		const total = steps.length;

		if (done === 0) {
			// Don't clutter UI when nothing started
			ctx.ui.setWidget("workflow", undefined);
			return;
		}

		const pct = Math.round((done / total) * 100);

		// Find current phase
		const current = steps.find((s) => !s.done);
		const currentInfo = current
			? `→ Step ${current.id}: ${current.label} [${phaseLabel(current.phase)}]`
			: "✓ Workflow complete!";

		ctx.ui.setWidget("workflow", (_tui, theme) => {
			const barLen = 15;
			const filled = Math.round((done / total) * barLen);
			const bar =
				theme.fg("success", "█".repeat(filled)) +
				theme.fg("dim", "░".repeat(barLen - filled));

			const line =
				theme.fg("accent", theme.bold("Workflow ")) +
				bar +
				theme.fg("muted", ` ${done}/${total} (${pct}%) `) +
				theme.fg("dim", currentInfo);

			return new Text(line, 0, 0);
		});
	};

	// ── Guard: block git commit if BDD/TDD steps incomplete ───────────

	pi.on("tool_call", async (event, ctx) => {
		if (!isToolCallEventType("bash", event)) return;

		const cmd = event.input.command?.trim() ?? "";

		// Only guard `git commit` (not push, not other git commands)
		if (!/\bgit\s+commit\b/.test(cmd)) return;

		// Steps 1-6 must be done before committing (RED → GREEN → REFACTOR → GREEN)
		const preCommitSteps = steps.filter((s) => s.id <= 6);
		const incomplete = preCommitSteps.filter((s) => !s.done);

		if (incomplete.length > 0) {
			const missing = incomplete.map((s) => `  ○ Step ${s.id}: ${s.label}`).join("\n");

			if (!ctx.hasUI) {
				return {
					block: true,
					reason: `Workflow violation: complete these steps before committing:\n${missing}`,
				};
			}

			const ok = await ctx.ui.confirm(
				"⚠️ Workflow: Incomplete Steps",
				`These BDD/TDD steps are not checked off:\n\n${missing}\n\nCommit anyway?`,
			);
			if (!ok) {
				return { block: true, reason: "Blocked by workflow extension — incomplete BDD/TDD steps" };
			}
		}
	});

	// ── Guard: enforce sharded BDD runs ───────────────────────────────

	pi.on("tool_call", async (event, ctx) => {
		if (!isToolCallEventType("bash", event)) return;

		const cmd = event.input.command?.trim() ?? "";

		// Detect `cargo test --test bdd` without shard env vars
		if (!/cargo\s+test\b/.test(cmd) || !/--test\s+bdd\b/.test(cmd)) return;

		// Allow if shard env vars are present (inline env or exported)
		if (/QUECTO_BDD_SHARD_INDEX/.test(cmd) && /QUECTO_BDD_SHARD_TOTAL/.test(cmd)) return;

		// Allow if running via the sharding script
		if (/run-bdd-shards\.sh/.test(cmd)) return;

		// Allow if QUECTO_TAG is set (single-scenario debugging with @focus)
		if (/QUECTO_TAG/.test(cmd)) return;

		const reason =
			"BDD tests must run sharded (24-way parallel). Use:\n" +
			"  bash scripts/run-bdd-shards.sh\n" +
			"Or for a single scenario, tag it @focus and run:\n" +
			"  QUECTO_TAG=focus cargo test --no-fail-fast --features test-support --test bdd 2>&1 | scripts/test-filter.sh";

		if (!ctx.hasUI) {
			return { block: true, reason };
		}

		const ok = await ctx.ui.confirm(
			"⚠️ Unsharded BDD Run Detected",
			`Running all BDD tests in a single process will be very slow.\n\n${reason}\n\nRun unsharded anyway?`,
		);
		if (!ok) {
			return { block: true, reason: "Blocked by workflow extension — use sharded BDD runs" };
		}
	});

	// ── System prompt injection ───────────────────────────────────────

	pi.on("before_agent_start", async (event, _ctx) => {
		const done = steps.filter((s) => s.done).length;
		const total = steps.length;
		const current = steps.find((s) => !s.done);

		let injection = `\n\n## Active Development Workflow (Quecto AGENTS.md)\n`;
		injection += `Progress: ${done}/${total} steps complete.\n`;

		if (current) {
			injection += `CURRENT STEP → ${current.id}. ${current.label} [${phaseLabel(current.phase)}]\n`;
			injection += `\nYou MUST follow the BDD/TDD Red-Green-Refactor process.\n`;
			injection += `Use the \`workflow\` tool to check off steps as you complete them.\n`;
			injection += `Do NOT skip ahead — complete steps in order.\n`;
		} else {
			injection += `All steps complete! You may start a new workflow cycle with \`workflow reset\`.\n`;
		}

		return { systemPrompt: event.systemPrompt + injection };
	});

	// ── Tool: LLM-callable workflow management ────────────────────────

	const WorkflowParams = Type.Object({
		action: StringEnum(["status", "check", "uncheck", "reset", "skip"] as const),
		step: Type.Optional(Type.Number({ description: "Step number (1-15)" })),
	});

	pi.registerTool({
		name: "workflow",
		label: "Workflow",
		description: [
			"Manage the Quecto BDD/TDD development workflow checklist.",
			"Actions:",
			"  status  — Show all steps and current progress",
			"  check   — Mark a step as done (requires step number)",
			"  uncheck — Unmark a step (requires step number)",
			"  reset   — Reset all steps for a new cycle",
			"  skip    — Mark a step as done even if previous steps are incomplete (requires step number)",
			"",
			"Steps should be completed in order. The workflow enforces:",
			"  1-3: RED phase (scenarios, tests, verify they fail)",
			"  4:   GREEN phase (implement until tests pass)",
			"  5:   REFACTOR phase (clean up)",
			"  6:   GREEN phase (verify tests still pass)",
			"  7-15: CI/CD and review",
		].join("\n"),
		parameters: WorkflowParams,

		async execute(_toolCallId, params, _signal, _onUpdate, ctx) {
			const makeDetails = (action: WorkflowDetails["action"], error?: string): WorkflowDetails => ({
				action,
				steps: steps.map((s) => ({ ...s })),
				error,
			});

			const formatStatus = (): string => {
				const done = steps.filter((s) => s.done).length;
				const total = steps.length;
				let text = `Workflow: ${done}/${total} complete\n\n`;
				let lastPhase = "";
				for (const s of steps) {
					if (s.phase !== lastPhase) {
						lastPhase = s.phase;
						text += `\n[${phaseLabel(s.phase)}]\n`;
					}
					text += `  [${s.done ? "x" : " "}] ${s.id}. ${s.label}\n`;
				}
				const current = steps.find((s) => !s.done);
				if (current) {
					text += `\n→ Next: Step ${current.id} — ${current.label}`;
				} else {
					text += `\n✓ All steps complete!`;
				}
				return text;
			};

			switch (params.action) {
				case "status": {
					updateWidget(ctx);
					return {
						content: [{ type: "text", text: formatStatus() }],
						details: makeDetails("status"),
					};
				}

				case "check": {
					if (params.step === undefined) {
						return {
							content: [{ type: "text", text: "Error: step number required" }],
							details: makeDetails("check", "step number required"),
						};
					}
					const step = steps.find((s) => s.id === params.step);
					if (!step) {
						return {
							content: [{ type: "text", text: `Error: no step #${params.step}` }],
							details: makeDetails("check", `no step #${params.step}`),
						};
					}
					// Warn if previous steps aren't done (but allow it)
					const prev = steps.filter((s) => s.id < step.id && !s.done);
					let warning = "";
					if (prev.length > 0) {
						warning = `\n⚠️ Warning: ${prev.length} earlier step(s) still incomplete`;
					}
					step.done = true;
					updateWidget(ctx);
					return {
						content: [{ type: "text", text: `✓ Step ${step.id} checked: ${step.label}${warning}` }],
						details: makeDetails("check"),
					};
				}

				case "uncheck": {
					if (params.step === undefined) {
						return {
							content: [{ type: "text", text: "Error: step number required" }],
							details: makeDetails("uncheck", "step number required"),
						};
					}
					const step = steps.find((s) => s.id === params.step);
					if (!step) {
						return {
							content: [{ type: "text", text: `Error: no step #${params.step}` }],
							details: makeDetails("uncheck", `no step #${params.step}`),
						};
					}
					step.done = false;
					updateWidget(ctx);
					return {
						content: [{ type: "text", text: `○ Step ${step.id} unchecked: ${step.label}` }],
						details: makeDetails("uncheck"),
					};
				}

				case "skip": {
					if (params.step === undefined) {
						return {
							content: [{ type: "text", text: "Error: step number required" }],
							details: makeDetails("skip", "step number required"),
						};
					}
					const step = steps.find((s) => s.id === params.step);
					if (!step) {
						return {
							content: [{ type: "text", text: `Error: no step #${params.step}` }],
							details: makeDetails("skip", `no step #${params.step}`),
						};
					}
					step.done = true;
					updateWidget(ctx);
					return {
						content: [{ type: "text", text: `⏭ Step ${step.id} skipped: ${step.label}` }],
						details: makeDetails("skip"),
					};
				}

				case "reset": {
					steps = freshSteps();
					updateWidget(ctx);
					return {
						content: [{ type: "text", text: "Workflow reset — all 15 steps cleared for new cycle" }],
						details: makeDetails("reset"),
					};
				}

				default:
					return {
						content: [{ type: "text", text: `Unknown action: ${params.action}` }],
						details: makeDetails("status", `unknown action: ${params.action}`),
					};
			}
		},

		renderCall(args, theme) {
			let text = theme.fg("toolTitle", theme.bold("workflow ")) + theme.fg("muted", args.action);
			if (args.step !== undefined) text += ` ${theme.fg("accent", `#${args.step}`)}`;
			return new Text(text, 0, 0);
		},

		renderResult(result, { expanded }, theme) {
			const details = result.details as WorkflowDetails | undefined;
			if (!details) {
				const text = result.content[0];
				return new Text(text?.type === "text" ? text.text : "", 0, 0);
			}

			if (details.error) {
				return new Text(theme.fg("error", `Error: ${details.error}`), 0, 0);
			}

			const done = details.steps.filter((s) => s.done).length;
			const total = details.steps.length;
			const pct = Math.round((done / total) * 100);

			switch (details.action) {
				case "status": {
					let text = theme.fg("muted", `${done}/${total} (${pct}%)`);
					const current = details.steps.find((s) => !s.done);
					if (current) {
						const colorFn = phaseColor(current.phase, theme);
						text += ` → ${colorFn(phaseLabel(current.phase))} Step ${current.id}`;
					} else {
						text += " " + theme.fg("success", "✓ Complete!");
					}
					if (expanded) {
						let lastPhase = "";
						for (const s of details.steps) {
							if (s.phase !== lastPhase) {
								lastPhase = s.phase;
								const colorFn = phaseColor(s.phase, theme);
								text += `\n  ${colorFn(theme.bold(phaseLabel(s.phase)))}`;
							}
							const check = s.done ? theme.fg("success", "✓") : theme.fg("dim", "○");
							const label = s.done ? theme.fg("dim", s.label) : theme.fg("text", s.label);
							text += `\n  ${check} ${theme.fg("accent", `${s.id}.`)} ${label}`;
						}
					}
					return new Text(text, 0, 0);
				}

				case "check":
				case "skip": {
					const msg = result.content[0];
					return new Text(theme.fg("success", "✓ ") + theme.fg("muted", msg?.type === "text" ? msg.text : ""), 0, 0);
				}

				case "uncheck": {
					const msg = result.content[0];
					return new Text(theme.fg("warning", "○ ") + theme.fg("muted", msg?.type === "text" ? msg.text : ""), 0, 0);
				}

				case "reset":
					return new Text(theme.fg("warning", "↺ ") + theme.fg("muted", "Workflow reset"), 0, 0);
			}
		},
	});

	// ── Command: /workflow — interactive checklist ─────────────────────

	pi.registerCommand("workflow", {
		description: "Open the Quecto BDD/TDD workflow checklist",
		handler: async (_args, ctx) => {
			if (!ctx.hasUI) {
				// Print mode fallback
				const done = steps.filter((s) => s.done).length;
				ctx.ui.notify(`Workflow: ${done}/${steps.length} steps complete`, "info");
				return;
			}

			const updatedSteps = await ctx.ui.custom<WorkflowStep[]>((_tui, theme, _kb, done) => {
				return new WorkflowChecklist(steps, theme, (result) => done(result));
			});

			if (updatedSteps) {
				// Apply changes from the interactive checklist
				steps = updatedSteps;

				// Persist via appendEntry so it survives restarts
				// (The tool results handle branching; this handles manual toggles)
				pi.appendEntry("workflow-state", { steps: steps.map((s) => ({ ...s })) });

				updateWidget(ctx);
			}
		},
	});

	// ── Shortcut: Ctrl+Shift+W to quickly open workflow ──────────────

	pi.registerShortcut("ctrl+shift+w", {
		description: "Open Quecto workflow checklist",
		handler: async (ctx) => {
			if (!ctx.hasUI) return;

			const updatedSteps = await ctx.ui.custom<WorkflowStep[]>((_tui, theme, _kb, done) => {
				return new WorkflowChecklist(steps, theme, (result) => done(result));
			});

			if (updatedSteps) {
				steps = updatedSteps;
				pi.appendEntry("workflow-state", { steps: steps.map((s) => ({ ...s })) });
				updateWidget(ctx);
			}
		},
	});
}
