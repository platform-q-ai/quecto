/**
 * Quecto Workflow Extension - Enforces the BDD/TDD Red-Green-Refactor development workflow
 * from AGENTS.md as an interactive todo checklist in Pi.
 *
 * The workflow has 14 steps:
 *  1. Update Scenarios / Add new features as necessary
 *  2. Write/update unit tests
 *  3. Ensure new/modified tests fail (RED)
 *  4. Implement code (GREEN)
 *  5. Commit
 *  6. Push (pre-push hook runs tests and linting)
 *  7. Create PR
 *  8. Despatch sub agents in parallel as reviewers (Architecture, Security, Performance)
 *  9. Fix all valid review concerns
 * 10. Push changes to remote
 * 11. Reply to reviewers comments and mark resolved (use graphql)
 * 12. Run pre-merge hooks (real-LLM, machete, deny)
 * 13. Merge
 * 14. Move to local master and pull
 *
 * Features:
 * - `/workflow` command opens an interactive checklist UI
 * - `workflow` tool lets the LLM check/uncheck/reset/query steps
 * - Tracks active GitHub issue (set/clear) and shows it in the widget
 * - Widget above editor shows current progress at a glance
 * - Blocks git commit if RED/GREEN/REFACTOR steps aren't done
 * - Injects workflow awareness into the system prompt
 * - Completion nudge prompts for the next issue when all steps are done
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

/** The active GitHub issue being worked on this cycle. */
interface ActiveIssue {
	number: number;
	title: string;
}

const WORKFLOW_TEMPLATE: Omit<WorkflowStep, "done">[] = [
	{ id: 1, label: "Update Scenarios / Add new features", phase: "red" },
	{ id: 2, label: "Write/update unit tests (run a quick smoke check; full suite runs on push)", phase: "red" },
	{ id: 3, label: "Ensure new/modified tests FAIL (RED) — quick targeted run only, not full suite", phase: "red" },
	{ id: 4, label: "Implement code (GREEN)", phase: "green" },
	{ id: 5, label: "Commit", phase: "ci" },
	{ id: 6, label: "Push (pre-push hook will run tests and linting)", phase: "ci" },
	{ id: 7, label: "Create PR", phase: "ci" },
	{ id: 8, label: "Despatch sub agents in parallel as reviewers (Architecture, Security and Performance)", phase: "review" },
	{ id: 9, label: "Fix all valid review concerns", phase: "review" },
	{ id: 10, label: "Push changes to remote", phase: "review" },
	{ id: 11, label: "Reply to the reviewers comments on the PR and mark resolved (use graphql)", phase: "review" },
	{ id: 12, label: "Run pre-merge hooks (real-LLM, machete, deny)", phase: "ci" },
	{ id: 13, label: "Merge", phase: "ci" },
	{ id: 14, label: "Move to local master and pull", phase: "ci" },
];

function freshSteps(): WorkflowStep[] {
	return WORKFLOW_TEMPLATE.map((s) => ({ ...s, done: false }));
}

// ─── Tool details shape (for state persistence) ───────────────────────

interface WorkflowDetails {
	action: "status" | "check" | "uncheck" | "reset" | "skip" | "set_issue" | "clear_issue";
	steps: WorkflowStep[];
	activeIssue?: ActiveIssue;
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
	private activeIssue: ActiveIssue | undefined;
	private theme: Theme;
	private onClose: (steps: WorkflowStep[]) => void;
	private selected: number = 0;
	private cachedWidth?: number;
	private cachedLines?: string[];

	constructor(
		steps: WorkflowStep[],
		activeIssue: ActiveIssue | undefined,
		theme: Theme,
		onClose: (steps: WorkflowStep[]) => void,
	) {
		this.steps = steps.map((s) => ({ ...s })); // deep copy
		this.activeIssue = activeIssue;
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
		if (this.activeIssue) {
			lines.push(truncateToWidth(
				`  ${th.fg("accent", th.bold(`Issue #${this.activeIssue.number}`))} ${th.fg("muted", this.activeIssue.title)}`,
				width,
			));
		}
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
	let activeIssue: ActiveIssue | undefined = undefined;
	let autoComplete = false;
	let completionNudgeEnabled = true;
	let completionNudgeFired = false;

	// ── State reconstruction ──────────────────────────────────────────

	const reconstructState = (ctx: ExtensionContext) => {
		steps = freshSteps();
		activeIssue = undefined;

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
						activeIssue = details.activeIssue;
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
				activeIssue = entry.data.activeIssue as ActiveIssue | undefined;
			}
		}

		completionNudgeFired = steps.every((s) => s.done);
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

		if (done === 0 && !activeIssue) {
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

			const issuePart = activeIssue
				? theme.fg("accent", theme.bold(` #${activeIssue.number}`)) +
				  theme.fg("dim", ` ${activeIssue.title.slice(0, 40)}${activeIssue.title.length > 40 ? "…" : ""} `)
				: " ";

			const line =
				theme.fg("accent", theme.bold("Workflow")) +
				issuePart +
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

		// Steps 1-4 must be done before committing (RED → GREEN)
		const preCommitSteps = steps.filter((s) => s.id <= 4);
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
				return { block: true, reason: "Blocked by workflow extension — incomplete RED/GREEN steps" };
			}
		}
	});

	// ── Guard: enforce sharded BDD runs ───────────────────────────────

	pi.on("tool_call", async (event, _ctx) => {
		if (!isToolCallEventType("bash", event)) return;

		const cmd = event.input.command?.trim() ?? "";

		// Detect `cargo test --test bdd`
		if (!/cargo\s+test\b/.test(cmd) || !/--test\s+bdd\b/.test(cmd)) return;

		// Allow if running via the sharding script
		if (/run-bdd-shards\.sh/.test(cmd)) return;

		// Allow if QUECTO_TAG is set (single-scenario debugging with @focus)
		if (/QUECTO_TAG/.test(cmd)) return;

		// Check shard env vars — must use exactly 24 shards
		const totalMatch = cmd.match(/QUECTO_BDD_SHARD_TOTAL=(\d+)/);
		if (totalMatch && /QUECTO_BDD_SHARD_INDEX/.test(cmd)) {
			const total = parseInt(totalMatch[1], 10);
			if (total === 24) return;
			// Reject any other shard count
			return {
				block: true,
				reason:
					`QUECTO_BDD_SHARD_TOTAL=${total} is not allowed. Use 24 shards:\n` +
					"  bash scripts/run-bdd-shards.sh",
			};
		}

		// No shard vars at all — block
		return {
			block: true,
			reason:
				"BDD tests must run sharded (24-way parallel). Use:\n" +
				"  bash scripts/run-bdd-shards.sh\n" +
				"Or for a single scenario, tag it @focus and run:\n" +
				"  QUECTO_TAG=focus cargo test --no-fail-fast --features test-support --test bdd 2>&1 | scripts/test-filter.sh",
		};
	});

	// ── System prompt injection ───────────────────────────────────────

	pi.on("before_agent_start", async (event, _ctx) => {
		const done = steps.filter((s) => s.done).length;
		const total = steps.length;
		const current = steps.find((s) => !s.done);

		let injection = `\n\n## Active Development Workflow (Quecto AGENTS.md)\n`;
		injection += `Progress: ${done}/${total} steps complete.\n`;
		if (activeIssue) {
			injection += `Active issue: #${activeIssue.number} — ${activeIssue.title}\n`;
		} else {
			injection += `Active issue: (not set) — call workflow(action="set_issue", issueNumber=<n>, issueTitle="...")\n`;
		}

		if (current) {
			injection += `CURRENT STEP → ${current.id}. ${current.label} [${phaseLabel(current.phase)}]\n`;
			injection += `\nYou MUST follow the BDD/TDD Red-Green-Refactor process.\n`;
			injection += `Use the \`workflow\` tool to check off steps as you complete them.\n`;
			injection += `Do NOT skip ahead — complete steps in order.\n`;

			// Emphasize full implementation on all RED→GREEN steps (1-4)
			if (current.id >= 1 && current.id <= 4) {
				injection += `\n**DO NOT WORRY ABOUT THE SIZE OF A CHANGE, IMPLEMENT IT IN FULL, DO NOT DEFER.**\n`;
			}

			// Step-specific instructions
			if (current.id === 2) {
				injection += `\n### Step 2: Write/update unit tests\n`;
				injection += `Write or update the unit tests and BDD scenarios for the change.\n`;
				injection += `Run a quick targeted smoke check to confirm they compile and are wired up:\n`;
				injection += "```bash\n";
				injection += `cargo test --no-fail-fast --lib -- <your_module> 2>&1 | scripts/test-filter.sh\n`;
				injection += "```\n";
				injection += `Do NOT run the full BDD suite here — the pre-push hook runs all 24 shards automatically on \`git push\`.\n`;
			}

			if (current.id === 3) {
				injection += `\n### Step 3: Verify tests FAIL (RED)\n`;
				injection += `Run only the new/modified tests to confirm they fail before any implementation:\n`;
				injection += "```bash\n";
				injection += `cargo test --no-fail-fast --lib -- <your_test_name> 2>&1 | scripts/test-filter.sh\n`;
				injection += "```\n";
				injection += `For a BDD scenario, tag it @focus and run:\n`;
				injection += "```bash\n";
				injection += `QUECTO_TAG=focus cargo test --no-fail-fast --features test-support --test bdd 2>&1 | scripts/test-filter.sh\n`;
				injection += "```\n";
				injection += `Do NOT run the full suite — the pre-push hook covers that. You only need to confirm the specific new tests are red.\n`;
			}

			if (current.id === 8) {
				injection += `\n### Step 8: Dispatch Reviewer Subagents\n`;
				injection += `Use the \`subagent\` tool in parallel mode to dispatch all three reviewers simultaneously.\n`;
				injection += `Each reviewer will submit a formal GitHub PR review with inline comments.\n`;
				injection += `\n**IMPORTANT: The exact agent names are (with hyphens, not underscores):**\n`;
				injection += `- \`architecture-reviewer\`\n`;
				injection += `- \`security-reviewer\`\n`;
				injection += `- \`performance-reviewer\`\n`;
				injection += `\nExample:\n`;
				injection += "```json\n";
				injection += `{\n`;
				injection += `  "tasks": [\n`;
				injection += `    { "agent": "architecture-reviewer", "task": "Review PR #<number> in this repo for architectural soundness, system design, modularity, and upstream compatibility. Submit a formal GitHub PR review with inline comments." },\n`;
				injection += `    { "agent": "security-reviewer", "task": "Review PR #<number> in this repo for security vulnerabilities, input validation, auth flaws, and data exposure risks. Submit a formal GitHub PR review with inline comments." },\n`;
				injection += `    { "agent": "performance-reviewer", "task": "Review PR #<number> in this repo for performance regressions, memory leaks, unbounded growth, and hot path efficiency. Submit a formal GitHub PR review with inline comments." }\n`;
				injection += `  ]\n`;
				injection += `}\n`;
				injection += "```\n";
				injection += `Get the PR number from \`gh pr view --json number -q .number\` or from the PR URL created in step 7.\n`;
				injection += `\nDo NOT use names like \`reviewer-architecture\` or \`reviewer_security\` — those do not exist.\n`;
			}

			if (current.id === 11) {
				injection += `\n### Step 11: Reply to Reviewer Comments and Resolve Threads\n`;
				injection += `Reply to every review comment on the PR, then resolve the threads.\n`;
				injection += `Use GraphQL exclusively — do NOT use REST API endpoints.\n`;
				injection += `\n**Step 1: Get the repo owner and name**\n`;
				injection += "```bash\n";
				injection += `gh repo view --json owner,name --jq '(.owner.login) + "/" + .name'\n`;
				injection += "```\n";
				injection += `\n**Step 2: List all review threads**\n`;
				injection += "```bash\n";
				injection += `gh api graphql -f query='\n`;
				injection += `query {\n`;
				injection += `  repository(owner: "OWNER", name: "REPO") {\n`;
				injection += `    pullRequest(number: PR_NUMBER) {\n`;
				injection += `      reviewThreads(first: 50) {\n`;
				injection += `        nodes {\n`;
				injection += `          id\n`;
				injection += `          isResolved\n`;
				injection += `          comments(first: 1) {\n`;
				injection += `            nodes { id body }\n`;
				injection += `          }\n`;
				injection += `        }\n`;
				injection += `      }\n`;
				injection += `    }\n`;
				injection += `  }\n`;
				injection += `}'\n`;
				injection += "```\n";
				injection += `\n**Step 3: Reply to each thread**\n`;
				injection += `The mutation is \`addPullRequestReviewThreadReply\`. The thread ID comes from step 2.\n`;
				injection += "```bash\n";
				injection += `gh api graphql -f query='\n`;
				injection += `mutation {\n`;
				injection += `  addPullRequestReviewThreadReply(input: {\n`;
				injection += `    pullRequestReviewThreadId: "PRRT_kwDO..."\n`;
				injection += `    body: "Fixed in <commit>. <explanation>"\n`;
				injection += `  }) {\n`;
				injection += `    comment { id }\n`;
				injection += `  }\n`;
				injection += `}'\n`;
				injection += "```\n";
				injection += `\n**Step 4: Resolve each thread**\n`;
				injection += "```bash\n";
				injection += `gh api graphql -f query='\n`;
				injection += `mutation {\n`;
				injection += `  resolveReviewThread(input: {\n`;
				injection += `    threadId: "PRRT_kwDO..."\n`;
				injection += `  }) {\n`;
				injection += `    thread { id isResolved }\n`;
				injection += `  }\n`;
				injection += `}'\n`;
				injection += "```\n";
				injection += `\n**CRITICAL RULES:**\n`;
				injection += `- Thread IDs look like \`PRRT_kwDO...\` — they come from the \`reviewThreads.nodes[].id\` field.\n`;
				injection += `- The reply mutation is \`addPullRequestReviewThreadReply\` — NOT \`addPullRequestReviewComment\`.\n`;
				injection += `  (\`addPullRequestReviewComment\` does NOT accept \`pullRequestReviewThreadId\` — it will error.)\n`;
				injection += `- Do NOT use REST API endpoints (\`/pulls/comments/\`, \`/replies\`, \`/reviews\`). Use GraphQL only.\n`;
				injection += `- Escape special characters in the body: backticks, quotes, dollar signs.\n`;
				injection += `  Use double-escaping in bash: \\\\\`backtick\\\\\`, or avoid problematic chars.\n`;
				injection += `- For bulk replies, iterate over thread IDs in a bash loop.\n`;
			}

			if (current.id === 12) {
				injection += `\n### Step 12: Run Pre-Merge Hooks\n`;
				injection += `Run the pre-merge-commit checks (real-LLM e2e tests, cargo machete, cargo deny).\n`;
				injection += `Use the sharded runner — do NOT run BDD tests in a single process:\n`;
				injection += "```bash\n";
				injection += `bash scripts/run-bdd-shards.sh --suite real-llm-bdd --shards 24 --timeout 12m --tag real-llm --real-llm\n`;
				injection += "```\n";
				injection += `Then run:\n`;
				injection += "```bash\n";
				injection += `cargo machete\n`;
				injection += `cargo deny check\n`;
				injection += "```\n";
				injection += `If OPENAI_API_KEY is not set, skip the real-LLM suite but still run machete and deny.\n`;
			}
		} else {
			injection += `All steps complete! You may start a new workflow cycle with \`workflow reset\`.\n`;
		}

		return { systemPrompt: event.systemPrompt + injection };
	});

	// ── Tool: LLM-callable workflow management ────────────────────────

	const WorkflowParams = Type.Object({
		action: StringEnum(["status", "check", "uncheck", "reset", "skip", "set_issue", "clear_issue"] as const),
		step: Type.Optional(Type.Number({ description: "Step number (1-14)" })),
		issueNumber: Type.Optional(Type.Number({ description: "GitHub issue number (required for set_issue)" })),
		issueTitle: Type.Optional(Type.String({ description: "GitHub issue title (required for set_issue)" })),
	});

	pi.registerTool({
		name: "workflow",
		label: "Workflow",
		description: [
			"Manage the Quecto BDD/TDD development workflow checklist.",
			"Actions:",
			"  status      — Show all steps and current progress",
			"  check       — Mark a step as done (requires step number)",
			"  uncheck     — Unmark a step (requires step number)",
			"  reset       — Reset all steps for a new cycle",
			"  skip        — Mark a step as done even if previous steps are incomplete (requires step number)",
			"  set_issue   — Record the GitHub issue this cycle is for (requires issueNumber + issueTitle)",
			"  clear_issue — Clear the active issue",
			"",
			"Steps should be completed in order. The workflow enforces:",
			"  1-3: RED phase (scenarios, tests, verify they fail)",
			"  4:   GREEN phase (implement until tests pass)",
			"  5:   REFACTOR phase (clean up)",
			"  6:   GREEN phase (verify tests still pass)",
			"  5-7: CI/CD (commit, push, PR)",
			"  8-11: Review",
			"  12-14: Pre-merge and merge",
		].join("\n"),
		parameters: WorkflowParams,

		async execute(_toolCallId, params, _signal, _onUpdate, ctx) {
			const makeDetails = (action: WorkflowDetails["action"], error?: string): WorkflowDetails => ({
				action,
				steps: steps.map((s) => ({ ...s })),
				activeIssue: activeIssue ? { ...activeIssue } : undefined,
				error,
			});

			const formatStatus = (): string => {
				const done = steps.filter((s) => s.done).length;
				const total = steps.length;
				let text = `Workflow: ${done}/${total} complete\n`;
				if (activeIssue) text += `Active issue: #${activeIssue.number} — ${activeIssue.title}\n`;
				text += "\n";
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

				case "set_issue": {
					if (params.issueNumber === undefined || !params.issueTitle) {
						return {
							content: [{ type: "text", text: "Error: issueNumber and issueTitle are required for set_issue" }],
							details: makeDetails("set_issue", "issueNumber and issueTitle required"),
						};
					}
					activeIssue = { number: params.issueNumber, title: params.issueTitle };
					pi.appendEntry("workflow-state", { steps: steps.map((s) => ({ ...s })), activeIssue: { ...activeIssue } });
					updateWidget(ctx);
					return {
						content: [{ type: "text", text: `🎯 Active issue set: #${activeIssue.number} — ${activeIssue.title}` }],
						details: makeDetails("set_issue"),
					};
				}

				case "clear_issue": {
					activeIssue = undefined;
					pi.appendEntry("workflow-state", { steps: steps.map((s) => ({ ...s })), activeIssue: undefined });
					updateWidget(ctx);
					return {
						content: [{ type: "text", text: "Active issue cleared" }],
						details: makeDetails("clear_issue"),
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
					completionNudgeFired = false;
					updateWidget(ctx);
					const issuePart = activeIssue
						? ` (still tracking issue #${activeIssue.number} — call set_issue once you have picked the next one)`
						: "";
					return {
						content: [{ type: "text", text: `Workflow reset — all ${steps.length} steps cleared for new cycle${issuePart}` }],
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
			if (args.issueNumber !== undefined) text += ` ${theme.fg("accent", `#${args.issueNumber}`)}`;
			if (args.issueTitle) text += ` ${theme.fg("dim", args.issueTitle)}`;
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
				case "set_issue": {
					const msg = result.content[0];
					return new Text(
						theme.fg("accent", "🎯 ") + theme.fg("muted", msg?.type === "text" ? msg.text : ""),
						0,
						0,
					);
				}
				case "clear_issue":
					return new Text(theme.fg("dim", "Issue cleared"), 0, 0);
				case "status": {
					let text = theme.fg("muted", `${done}/${total} (${pct}%)`);
					if (details.activeIssue) {
						text += theme.fg("accent", ` #${details.activeIssue.number}`);
					}
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
				const issuePart = activeIssue ? ` | Issue #${activeIssue.number}` : "";
				ctx.ui.notify(`Workflow: ${done}/${steps.length} steps complete${issuePart}`, "info");
				return;
			}

			const updatedSteps = await ctx.ui.custom<WorkflowStep[]>((_tui, theme, _kb, done) => {
				return new WorkflowChecklist(steps, activeIssue, theme, (result) => done(result));
			});

			if (updatedSteps) {
				// Apply changes from the interactive checklist
				steps = updatedSteps;

				// Persist via appendEntry so it survives restarts
				// (The tool results handle branching; this handles manual toggles)
				pi.appendEntry("workflow-state", { steps: steps.map((s) => ({ ...s })), activeIssue: activeIssue ? { ...activeIssue } : undefined });

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
				return new WorkflowChecklist(steps, activeIssue, theme, (result) => done(result));
			});

			if (updatedSteps) {
				steps = updatedSteps;
				pi.appendEntry("workflow-state", { steps: steps.map((s) => ({ ...s })), activeIssue: activeIssue ? { ...activeIssue } : undefined });
				updateWidget(ctx);
			}
		},
	});

	// ── Auto-complete + completion nudge ─────────────────────────────

	pi.on("agent_end", async (event, ctx) => {
		// Detect if the agent was aborted (ESC). The last assistant message
		// will have stopReason "aborted". Never send follow-ups after an
		// abort — respect the user's intent to stop.
		const lastMsg = event.messages[event.messages.length - 1];
		const wasAborted = event.messages.length === 0
			|| (lastMsg as any)?.stopReason === "aborted";
		if (wasAborted) return;

		// Don't nudge if the agent already has queued messages (it's continuing on its own).
		if (ctx.hasPendingMessages()) return;

		const allDone = steps.every((s) => s.done);

		if (autoComplete && !allDone) {
			const done = steps.filter((s) => s.done).length;
			const total = steps.length;

			// At least one step must have been checked to avoid nudging on a
			// fresh session where the agent hasn't started the workflow yet.
			if (done === 0) return;

			const current = steps.find((s) => !s.done);
			if (!current) return;

			const msg =
				`Workflow incomplete (${done}/${total}). ` +
				`Continue with the next incomplete step. ` +
				`Use the workflow tool to check off steps as you complete them. ` +
				`Respond with just the word DONE (no other text) when all ${total} steps are checked off.`;

			pi.sendUserMessage(msg, { deliverAs: "followUp" });
			return;
		}

		// Auto-disable autoComplete when all steps are done.
		if (autoComplete && allDone) {
			autoComplete = false;
			ctx.ui.notify("Workflow auto-continue OFF — all steps complete", "success");
		}

		if (allDone && !completionNudgeFired && completionNudgeEnabled) {
			completionNudgeFired = true;
			const issueLine = activeIssue
				? `You have completed all ${steps.length} workflow steps for issue #${activeIssue.number}: "${activeIssue.title}". `
				: `You have completed all ${steps.length} workflow steps. `;

			pi.sendUserMessage(
				issueLine +
				"Now do the following in order:\n" +
				"1. Close the issue (if applicable)\n" +
				"2. Pick the next issue to work on — if no open issues exist, respond with just the word NONE\n" +
				"3. Record it: call the workflow tool with action=\"set_issue\", issueNumber=<n>, issueTitle=\"...\"\n" +
				"4. Reset the checklist: call the workflow tool with action=\"reset\"\n" +
				"5. Begin Step 1 immediately for the new issue",
				{ deliverAs: "followUp" },
			);
		}

		if (!allDone) {
			completionNudgeFired = false;
		}
	});

	// ── Command: /workflow-auto — toggle auto-complete ────────────────

	pi.registerCommand("workflow-auto", {
		description: "Toggle auto-continue: nudge agent to keep going until all workflow steps are done",
		handler: async (_args, ctx) => {
			autoComplete = !autoComplete;
			ctx.ui.notify(
				autoComplete
					? "Workflow auto-continue ON — agent will be nudged to complete all steps"
					: "Workflow auto-continue OFF",
				"info",
			);
		},
	});

	// ── Shortcut: Ctrl+Shift+A to toggle auto-complete ───────────────

	pi.registerShortcut("ctrl+shift+a", {
		description: "Toggle workflow auto-continue",
		handler: async (ctx) => {
			autoComplete = !autoComplete;
			ctx.ui.notify(
				autoComplete
					? "Workflow auto-continue ON — agent will be nudged to complete all steps"
					: "Workflow auto-continue OFF",
				"info",
			);
		},
	});

	// ── Command: /workflow-nudge — toggle completion nudge ───────────

	pi.registerCommand("workflow-nudge", {
		description: "Toggle completion nudge: prompt agent to pick next issue when all steps are done",
		handler: async (_args, ctx) => {
			completionNudgeEnabled = !completionNudgeEnabled;
			ctx.ui.notify(
				completionNudgeEnabled
					? "Workflow completion nudge ON — agent will be prompted to pick next issue on cycle complete"
					: "Workflow completion nudge OFF — agent will stop after final step",
				"info",
			);
		},
	});

	// ── Shortcut: Ctrl+Shift+N to toggle completion nudge ───────────

	pi.registerShortcut("ctrl+shift+n", {
		description: "Toggle workflow completion nudge",
		handler: async (ctx) => {
			completionNudgeEnabled = !completionNudgeEnabled;
			ctx.ui.notify(
				completionNudgeEnabled
					? "Workflow completion nudge ON"
					: "Workflow completion nudge OFF",
				"info",
			);
		},
	});
}
