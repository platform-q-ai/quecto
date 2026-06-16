import { spawn } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { getMarkdownTheme, withFileMutationQueue } from "@earendil-works/pi-coding-agent";
import { Container, Markdown, Spacer, Text } from "@earendil-works/pi-tui";
import { Type } from "typebox";

type UsageStats = {
	input: number;
	output: number;
	cacheRead: number;
	cacheWrite: number;
	cost: number;
	turns: number;
};

type PanelModel = {
	name: string;
	model: string;
};

type FusionMode = "readonly" | "full" | "sandbox" | "patch-chain";

type PiRunResult = {
	name: string;
	model: string;
	exitCode: number;
	output: string;
	toolTrace: string;
	stderr: string;
	stopReason?: string;
	errorMessage?: string;
	usage: UsageStats;
	durationMs: number;
	sandboxCwd?: string;
	sandboxDiff?: string;
};

type FusionDetails = {
	prompt: string;
	mode: FusionMode;
	panel: PiRunResult[];
	analysis?: PiRunResult;
	final?: PiRunResult;
	models: {
		panel: PanelModel[];
		judge: string;
		synthesizer: string;
	};
};

type FusionProgressStatus = "pending" | "running" | "done" | "failed";

type FusionProgressItem = {
	name: string;
	model: string;
	status: FusionProgressStatus;
	durationMs?: number;
	usage?: UsageStats;
};

type FusionProgress = {
	stage: "panel" | "judge" | "synthesizer" | "done" | "failed";
	message: string;
	panel: FusionProgressItem[];
	judge: FusionProgressItem;
	synthesizer: FusionProgressItem;
};

const PANEL_MODELS: PanelModel[] = parsePanelModels(process.env.OSS_FUSION_MODELS) ?? [
	{ name: "minimax-m3", model: process.env.OSS_FUSION_MINIMAX_MODEL ?? "minimax/MiniMax-M3" },
	{
		name: "kimi-k2.7-fast",
		model: process.env.OSS_FUSION_KIMI_MODEL ?? "fireworks/accounts/fireworks/routers/kimi-k2p7-code-fast",
	},
	{
		name: "llama-cpp-qwen36-mtp",
		model:
			process.env.OSS_FUSION_QWEN_MODEL ??
			"llama-cpp-qwen36-mtp/qwen3.6-35b-a3b-mtp-iq4xs-q8nextn",
	},
];

const KIMI_K2P7_FAST_MODEL = "fireworks/accounts/fireworks/routers/kimi-k2p7-code-fast";
const JUDGE_MODEL = process.env.OSS_FUSION_JUDGE_MODEL ?? KIMI_K2P7_FAST_MODEL;
const SYNTHESIZER_MODEL = process.env.OSS_FUSION_SYNTHESIZER_MODEL ?? KIMI_K2P7_FAST_MODEL;
const THINKING_LEVEL = process.env.OSS_FUSION_THINKING ?? "high";
const CHILD_TIMEOUT_MS = Number(process.env.OSS_FUSION_TIMEOUT_MS ?? 30 * 60 * 1000);
const MAX_PANEL_OUTPUT_CHARS = Number(process.env.OSS_FUSION_MAX_PANEL_OUTPUT_CHARS ?? 80_000);
const MAX_SANDBOX_DIFF_CHARS = Number(process.env.OSS_FUSION_MAX_SANDBOX_DIFF_CHARS ?? 120_000);
const DEFAULT_FUSION_MODE = parseFusionMode(process.env.OSS_FUSION_MODE) ?? "readonly";
const READONLY_PANEL_TOOLS = ["read", "grep", "find", "ls", "webfetch", "brave_search"];
const FULL_PANEL_TOOLS = ["read", "bash", "edit", "write", "grep", "find", "ls", "webfetch", "brave_search"];
const SANDBOX_EXCLUDE_NAMES = new Set([
	".git",
	"node_modules",
	".next",
	"dist",
	"build",
	"target",
	".venv",
	"venv",
	"__pycache__",
	".cache",
	".turbo",
]);

function parseFusionMode(raw: string | undefined): FusionMode | undefined {
	const value = raw?.trim().toLowerCase();
	if (value === "readonly" || value === "full" || value === "sandbox" || value === "patch-chain") return value;
	return undefined;
}

function getToolsForMode(mode: FusionMode): string[] {
	return mode === "readonly" ? READONLY_PANEL_TOOLS : FULL_PANEL_TOOLS;
}

function isSandboxLikeMode(mode: FusionMode): boolean {
	return mode === "sandbox" || mode === "patch-chain";
}

function parsePanelModels(raw: string | undefined): PanelModel[] | undefined {
	if (!raw?.trim()) return undefined;
	const models = raw
		.split(",")
		.map((entry, index) => {
			const trimmed = entry.trim();
			if (!trimmed) return undefined;
			const eq = trimmed.indexOf("=");
			if (eq === -1) return { name: `model-${index + 1}`, model: trimmed };
			return { name: trimmed.slice(0, eq).trim() || `model-${index + 1}`, model: trimmed.slice(eq + 1).trim() };
		})
		.filter((m): m is PanelModel => Boolean(m?.model));
	return models.length > 0 ? models : undefined;
}

function getPiInvocation(args: string[]): { command: string; args: string[] } {
	const currentScript = process.argv[1];
	const isBunVirtualScript = currentScript?.startsWith("/$bunfs/root/");
	if (currentScript && !isBunVirtualScript && fs.existsSync(currentScript)) {
		return { command: process.execPath, args: [currentScript, ...args] };
	}

	const execName = path.basename(process.execPath).toLowerCase();
	const isGenericRuntime = /^(node|bun)(\.exe)?$/.test(execName);
	if (!isGenericRuntime) return { command: process.execPath, args };
	return { command: "pi", args };
}

async function writePromptToTempFile(prefix: string, prompt: string): Promise<{ dir: string; filePath: string }> {
	const tmpDir = await fs.promises.mkdtemp(path.join(os.tmpdir(), "pi-oss-fusion-"));
	const filePath = path.join(tmpDir, `${prefix.replace(/[^\w.-]+/g, "_")}.md`);
	await withFileMutationQueue(filePath, async () => {
		await fs.promises.writeFile(filePath, prompt, { encoding: "utf-8", mode: 0o600 });
	});
	return { dir: tmpDir, filePath };
}

function extractText(message: any): string {
	const content = message?.content;
	if (!Array.isArray(content)) return "";
	return content
		.filter((part: any) => part?.type === "text" && typeof part.text === "string")
		.map((part: any) => part.text)
		.join("\n")
		.trim();
}

function truncate(text: string, maxChars: number): string {
	if (text.length <= maxChars) return text;
	return `${text.slice(0, maxChars)}\n\n[truncated: ${text.length - maxChars} chars omitted]`;
}

function emptyUsage(): UsageStats {
	return { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0, turns: 0 };
}

function shouldExcludeFromSandbox(src: string): boolean {
	return SANDBOX_EXCLUDE_NAMES.has(path.basename(src));
}

async function createSandbox(sourceCwd: string, panelName: string): Promise<string> {
	const root = await fs.promises.mkdtemp(path.join(os.tmpdir(), `pi-oss-fusion-${panelName.replace(/[^\w.-]+/g, "_")}-`));
	await fs.promises.cp(sourceCwd, root, {
		recursive: true,
		force: true,
		filter: (src) => !shouldExcludeFromSandbox(src),
	});
	return root;
}

async function collectCommandOutput(command: string, args: string[], cwd: string, maxChars: number): Promise<string> {
	return await new Promise((resolve) => {
		const proc = spawn(command, args, { cwd, shell: false, stdio: ["ignore", "pipe", "pipe"] });
		let output = "";
		let truncated = false;
		const append = (text: string) => {
			if (output.length >= maxChars) {
				truncated = true;
				return;
			}
			const remaining = maxChars - output.length;
			output += text.slice(0, remaining);
			if (text.length > remaining) truncated = true;
		};
		const timeout = setTimeout(() => {
			truncated = true;
			proc.kill("SIGTERM");
		}, 30_000);
		timeout.unref();
		proc.stdout.on("data", (data) => append(data.toString()));
		proc.stderr.on("data", (data) => append(data.toString()));
		proc.on("close", () => {
			clearTimeout(timeout);
			resolve(truncated ? `${output}\n\n[diff truncated]` : output);
		});
		proc.on("error", (error) => {
			clearTimeout(timeout);
			resolve(error instanceof Error ? error.message : String(error));
		});
	});
}

async function collectSandboxDiff(sourceCwd: string, sandboxCwd: string): Promise<string> {
	const excludeArgs = Array.from(SANDBOX_EXCLUDE_NAMES).flatMap((name) => ["--exclude", name]);
	const diff = await collectCommandOutput(
		"diff",
		["-ruN", ...excludeArgs, sourceCwd, sandboxCwd],
		path.dirname(sourceCwd),
		MAX_SANDBOX_DIFF_CHARS,
	);
	return diff.trim();
}

async function runPiPrompt(options: {
	name: string;
	model: string;
	prompt: string;
	cwd: string;
	tools?: string[];
	signal?: AbortSignal;
	thinking?: string;
}): Promise<PiRunResult> {
	const started = Date.now();
	const usage = emptyUsage();
	const tmp = await writePromptToTempFile(options.name, options.prompt);
	const args = ["--mode", "json", "-p", "--no-session", "--model", options.model];

	if (options.thinking) args.push("--thinking", options.thinking);
	if (options.tools?.length) args.push("--tools", options.tools.join(","));
	else args.push("--no-tools");
	args.push(`@${tmp.filePath}`);

	let output = "";
	let toolTrace = "";
	let stderr = "";
	let stopReason: string | undefined;
	let errorMessage: string | undefined;
	let wasAborted = false;

	try {
		const exitCode = await new Promise<number>((resolve) => {
			const invocation = getPiInvocation(args);
			const proc = spawn(invocation.command, invocation.args, {
				cwd: options.cwd,
				shell: false,
				stdio: ["ignore", "pipe", "pipe"],
				env: { ...process.env, PI_SKIP_VERSION_CHECK: process.env.PI_SKIP_VERSION_CHECK ?? "1" },
			});

			let stdoutBuffer = "";
			const timeout = setTimeout(() => {
				wasAborted = true;
				stderr += `\nTimed out after ${CHILD_TIMEOUT_MS}ms.`;
				proc.kill("SIGTERM");
				setTimeout(() => proc.kill("SIGKILL"), 5000).unref();
			}, CHILD_TIMEOUT_MS);
			timeout.unref();

			const processLine = (line: string) => {
				if (!line.trim()) return;
				let event: any;
				try {
					event = JSON.parse(line);
				} catch {
					return;
				}

				if (event.type === "message_end" && event.message?.role === "assistant") {
					const text = extractText(event.message);
					if (text) output = text;

					const content = event.message.content;
					if (Array.isArray(content)) {
						for (const part of content) {
							if (part?.type === "toolCall") {
								toolTrace += `\n\n[assistant tool_call] ${part.name} ${truncate(JSON.stringify(part.arguments ?? {}), 4000)}`;
							}
						}
					}

					usage.turns++;
					const msgUsage = event.message.usage;
					if (msgUsage) {
						usage.input += msgUsage.input || 0;
						usage.output += msgUsage.output || 0;
						usage.cacheRead += msgUsage.cacheRead || 0;
						usage.cacheWrite += msgUsage.cacheWrite || 0;
						usage.cost += msgUsage.cost?.total || 0;
					}
					if (event.message.stopReason) stopReason = event.message.stopReason;
					if (event.message.errorMessage) errorMessage = event.message.errorMessage;
				}

				if (event.type === "message_end" && event.message?.role === "toolResult") {
					const text = extractText(event.message);
					const toolName = event.message.toolName ?? "tool";
					if (text) toolTrace += `\n\n[tool_result:${toolName}] ${truncate(text, 12000)}`;
				}
			};

			proc.stdout.on("data", (data) => {
				stdoutBuffer += data.toString();
				const lines = stdoutBuffer.split("\n");
				stdoutBuffer = lines.pop() ?? "";
				for (const line of lines) processLine(line);
			});

			proc.stderr.on("data", (data) => {
				stderr += data.toString();
			});

			proc.on("close", (code) => {
				clearTimeout(timeout);
				if (stdoutBuffer.trim()) processLine(stdoutBuffer);
				resolve(code ?? 0);
			});

			proc.on("error", (error) => {
				clearTimeout(timeout);
				stderr += error instanceof Error ? error.message : String(error);
				resolve(1);
			});

			const abort = () => {
				wasAborted = true;
				proc.kill("SIGTERM");
				setTimeout(() => proc.kill("SIGKILL"), 5000).unref();
			};
			if (options.signal?.aborted) abort();
			else options.signal?.addEventListener("abort", abort, { once: true });
		});

		return {
			name: options.name,
			model: options.model,
			exitCode: wasAborted && exitCode === 0 ? 1 : exitCode,
			output: output || errorMessage || stderr.trim() || "(no output)",
			toolTrace: toolTrace.trim(),
			stderr,
			stopReason: wasAborted ? "aborted" : stopReason,
			errorMessage,
			usage,
			durationMs: Date.now() - started,
		};
	} finally {
		try {
			await fs.promises.rm(tmp.dir, { recursive: true, force: true });
		} catch {
			// ignore cleanup failures
		}
	}
}

function buildPanelPrompt(userPrompt: string, mode: FusionMode): string {
	const modeInstruction =
		mode === "readonly"
			? "Mode: readonly. Inspect and research only. You do not have mutation tools; propose changes but do not edit files."
			: mode === "sandbox"
				? "Mode: sandbox. You may edit files and run commands, but only inside this temporary sandbox working directory. Do not write outside the current working directory. Summarize any changes you made."
				: mode === "patch-chain"
					? "Mode: patch-chain. You are one step in a sequential patch-refinement chain running in a temporary sandbox. Review the current sandbox state, preserve good prior changes, improve the implementation, and leave a coherent final patch. Do not write outside the current working directory."
					: "Mode: full. You may edit the real working tree and run commands. Use this power carefully and avoid unnecessary changes.";
	return [
		"You are one participant in an OSS Fusion panel running inside pi.",
		"Work independently. Do not assume the other panel models' answers.",
		modeInstruction,
		"Use the available tools when they help answer or complete the user's request.",
		"You have access to the tools enabled for this mode, including webfetch and brave_search when available.",
		"For current/latest/recent news or factual claims, do not answer from memory first: use brave_search and/or webfetch to retrieve live information from at least two sources, then cite what you found. If web tools fail, report the exact tool errors.",
		"Do not ask the user follow-up questions unless absolutely necessary.",
		"Favor specific evidence, URLs, file paths, commands inspected, and uncertainty over verbosity.",
		"",
		"Return this structure:",
		"## Answer",
		"## Evidence / Reasoning",
		"## Risks, Caveats, or Unknowns",
		"",
		"User request:",
		userPrompt,
	].join("\n");
}

function buildPatchChainPrompt(userPrompt: string, stepIndex: number, previousResults: PiRunResult[]): string {
	const previousSummary = previousResults.length
		? [
				"Previous chain steps already ran in this same sandbox. Their summaries and cumulative diffs follow.",
				formatPanelForPrompt(previousResults),
			].join("\n\n")
		: "This is the first patch-chain step. Create the initial implementation in the sandbox.";
	return [
		buildPanelPrompt(userPrompt, "patch-chain"),
		"",
		`Patch-chain step: ${stepIndex + 1}/${PANEL_MODELS.length}`,
		previousSummary,
		"",
		"Instructions for this step:",
		"- Inspect the current sandbox before editing.",
		"- If prior changes are good, keep them; if they are flawed, improve or replace them.",
		"- Run feasible checks.",
		"- End with a concise summary of what you changed and remaining risks.",
	].join("\n");
}

function formatPanelForPrompt(results: PiRunResult[]): string {
	return results
		.map((result) => {
			const status = result.exitCode === 0 && result.stopReason !== "error" ? "ok" : "failed";
			return [
				`## ${result.name}`,
				`model: ${result.model}`,
				`status: ${status}${result.stopReason ? ` (${result.stopReason})` : ""}`,
				result.sandboxCwd ? `sandbox: ${result.sandboxCwd}` : "",
				"",
				"### Final answer",
				truncate(result.output, MAX_PANEL_OUTPUT_CHARS),
				result.sandboxDiff ? "\n### Sandbox diff\n```diff\n" + truncate(result.sandboxDiff, MAX_PANEL_OUTPUT_CHARS) + "\n```" : "",
				result.toolTrace ? "\n### Tool transcript\n" + truncate(result.toolTrace, MAX_PANEL_OUTPUT_CHARS) : "",
			].filter(Boolean).join("\n");
		})
		.join("\n\n---\n\n");
}

function buildJudgePrompt(userPrompt: string, panelResults: PiRunResult[]): string {
	return [
		"You are the judge in an OSS Fusion pipeline.",
		"Read the user's request and all panel responses. Extract structure; do not write the final answer yet.",
		"Be critical: identify agreement, contradictions, missing coverage, weak evidence, hallucination risks, and uniquely useful insights. Treat uncited current-event claims as unreliable; prefer panel answers with tool transcripts and URLs.",
		"",
		"Return this structure:",
		"## Consensus",
		"## Contradictions / Disagreements",
		"## Unique Useful Insights By Model",
		"## Gaps / Blind Spots",
		"## Reliability Notes",
		"## Recommended Synthesis Plan",
		"",
		"User request:",
		userPrompt,
		"",
		"Panel responses:",
		formatPanelForPrompt(panelResults),
	].join("\n");
}

function buildSynthPrompt(userPrompt: string, panelResults: PiRunResult[], analysis: string): string {
	return [
		"You are the synthesizer in an OSS Fusion pipeline.",
		"Write the final answer to the user, grounded in the judge analysis, panel responses, and any included tool transcripts.",
		"Use consensus when available. Resolve disagreements explicitly. Preserve important caveats. For current-event questions, cite retrieved URLs/source names when available and do not hide that evidence behind a generic refusal.",
		"Do not mention the internal pipeline unless it helps clarify uncertainty.",
		"Be concise but complete.",
		"",
		"User request:",
		userPrompt,
		"",
		"Judge analysis:",
		analysis,
		"",
		"Panel responses:",
		formatPanelForPrompt(panelResults),
	].join("\n");
}

function formatUsage(usage: UsageStats): string {
	const parts: string[] = [];
	if (usage.turns) parts.push(`${usage.turns} turns`);
	if (usage.input) parts.push(`↑${usage.input}`);
	if (usage.output) parts.push(`↓${usage.output}`);
	if (usage.cacheRead) parts.push(`R${usage.cacheRead}`);
	if (usage.cacheWrite) parts.push(`W${usage.cacheWrite}`);
	if (usage.cost) parts.push(`$${usage.cost.toFixed(4)}`);
	return parts.join(" ");
}

function summarizeDetails(details: FusionDetails): string {
	const lines = ["# OSS Fusion", "", `Mode: ${details.mode}`, `Panel: ${details.panel.length} models`, ""];
	for (const result of details.panel) {
		const ok = result.exitCode === 0 && result.stopReason !== "error";
		lines.push(`- ${ok ? "✓" : "✗"} ${result.name} (${result.model}) ${formatUsage(result.usage)}`.trim());
	}
	if (details.analysis) lines.push(`- Judge: ${details.analysis.model} ${formatUsage(details.analysis.usage)}`.trim());
	if (details.final) lines.push(`- Synthesizer: ${details.final.model} ${formatUsage(details.final.usage)}`.trim());
	return lines.join("\n");
}

function statusIcon(status: FusionProgressStatus): string {
	if (status === "done") return "✓";
	if (status === "failed") return "✗";
	if (status === "running") return "◐";
	return "○";
}

function formatDuration(ms: number | undefined): string {
	if (!ms) return "";
	if (ms < 1000) return `${ms}ms`;
	return `${Math.round(ms / 1000)}s`;
}

function cloneProgress(progress: FusionProgress, message?: string): FusionProgress {
	return {
		stage: progress.stage,
		message: message ?? progress.message,
		panel: progress.panel.map((item) => ({ ...item, usage: item.usage ? { ...item.usage } : undefined })),
		judge: { ...progress.judge, usage: progress.judge.usage ? { ...progress.judge.usage } : undefined },
		synthesizer: {
			...progress.synthesizer,
			usage: progress.synthesizer.usage ? { ...progress.synthesizer.usage } : undefined,
		},
	};
}

function formatProgressLines(progress: FusionProgress): string[] {
	const lineFor = (label: string, item: FusionProgressItem) => {
		const parts = [`${statusIcon(item.status)} ${label}`, item.status];
		const duration = formatDuration(item.durationMs);
		if (duration) parts.push(duration);
		const usage = item.usage ? formatUsage(item.usage) : "";
		if (usage) parts.push(usage);
		return parts.join("  ");
	};

	return [
		`OSS Fusion — ${progress.message}`,
		...progress.panel.map((item) => lineFor(item.name, item)),
		lineFor("judge", progress.judge),
		lineFor("synthesizer", progress.synthesizer),
	];
}

function compactProgressStatus(progress: FusionProgress): string {
	const done = progress.panel.filter((item) => item.status === "done" || item.status === "failed").length;
	return `Fusion: ${progress.stage} (${done}/${progress.panel.length} panel)`;
}

async function runFusion(
	prompt: string,
	cwd: string,
	mode: FusionMode,
	signal: AbortSignal | undefined,
	onProgress?: (progress: FusionProgress) => void,
): Promise<{ text: string; details: FusionDetails; isError?: boolean }> {
	const progress: FusionProgress = {
		stage: "panel",
		message: `running ${mode} panel 0/${PANEL_MODELS.length}`,
		panel: PANEL_MODELS.map((model) => ({ name: model.name, model: model.model, status: "pending" })),
		judge: { name: "judge", model: JUDGE_MODEL, status: "pending" },
		synthesizer: { name: "synthesizer", model: SYNTHESIZER_MODEL, status: "pending" },
	};
	const emit = (message?: string) => onProgress?.(cloneProgress(progress, message));

	emit();
	let panel: PiRunResult[];
	if (mode === "patch-chain") {
		const chainSandbox = await createSandbox(cwd, "patch-chain");
		panel = [];
		try {
			for (let index = 0; index < PANEL_MODELS.length; index++) {
				const panelModel = PANEL_MODELS[index];
				progress.panel[index].status = "running";
				progress.message = `patch-chain step ${index + 1}/${PANEL_MODELS.length}: ${panelModel.name}`;
				emit();

				const result = await runPiPrompt({
					name: panelModel.name,
					model: panelModel.model,
					prompt: buildPatchChainPrompt(prompt, index, panel),
					cwd: chainSandbox,
					tools: getToolsForMode(mode),
					signal,
					thinking: THINKING_LEVEL,
				});

				result.sandboxCwd = chainSandbox;
				result.sandboxDiff = await collectSandboxDiff(cwd, chainSandbox);
				panel.push(result);
				progress.panel[index].status = result.exitCode === 0 && result.stopReason !== "error" ? "done" : "failed";
				progress.panel[index].durationMs = result.durationMs;
				progress.panel[index].usage = result.usage;
				progress.message = `patch-chain step ${index + 1}/${PANEL_MODELS.length} complete`;
				emit();
			}
		} finally {
			if (!process.env.OSS_FUSION_KEEP_SANDBOX) {
				await fs.promises.rm(chainSandbox, { recursive: true, force: true }).catch(() => undefined);
				for (const result of panel) result.sandboxCwd = `${chainSandbox} (deleted)`;
			}
		}
	} else {
		panel = await Promise.all(
			PANEL_MODELS.map(async (panelModel, index) => {
				progress.panel[index].status = "running";
				progress.message = `running ${mode} panel ${progress.panel.filter((item) => item.status === "done" || item.status === "failed").length}/${PANEL_MODELS.length}`;
				emit();

				let runCwd = cwd;
				let sandboxCwd: string | undefined;
				try {
					if (mode === "sandbox") {
						progress.message = `creating sandbox for ${panelModel.name}`;
						emit();
						sandboxCwd = await createSandbox(cwd, panelModel.name);
						runCwd = sandboxCwd;
					}

					const result = await runPiPrompt({
						name: panelModel.name,
						model: panelModel.model,
						prompt: buildPanelPrompt(prompt, mode),
						cwd: runCwd,
						tools: getToolsForMode(mode),
						signal,
						thinking: THINKING_LEVEL,
					});

					if (sandboxCwd) {
						result.sandboxCwd = sandboxCwd;
						result.sandboxDiff = await collectSandboxDiff(cwd, sandboxCwd);
						if (!process.env.OSS_FUSION_KEEP_SANDBOX) {
							await fs.promises.rm(sandboxCwd, { recursive: true, force: true });
							result.sandboxCwd = `${sandboxCwd} (deleted)`;
						}
					}

					progress.panel[index].status = result.exitCode === 0 && result.stopReason !== "error" ? "done" : "failed";
					progress.panel[index].durationMs = result.durationMs;
					progress.panel[index].usage = result.usage;
					progress.message = `${mode} panel ${progress.panel.filter((item) => item.status === "done" || item.status === "failed").length}/${PANEL_MODELS.length} complete`;
					emit();
					return result;
				} catch (error) {
					if (sandboxCwd && !process.env.OSS_FUSION_KEEP_SANDBOX) {
						await fs.promises.rm(sandboxCwd, { recursive: true, force: true }).catch(() => undefined);
					}
					const result: PiRunResult = {
						name: panelModel.name,
						model: panelModel.model,
						exitCode: 1,
						output: error instanceof Error ? error.message : String(error),
						toolTrace: "",
						stderr: "",
						stopReason: "error",
						errorMessage: error instanceof Error ? error.message : String(error),
						usage: emptyUsage(),
						durationMs: 0,
						sandboxCwd,
					};
					progress.panel[index].status = "failed";
					emit(`${panelModel.name} failed`);
					return result;
				}
			}),
		);
	}

	const usablePanel = panel.filter((r) => r.exitCode === 0 && r.stopReason !== "error" && r.output.trim());
	const details: FusionDetails = {
		prompt,
		mode,
		panel,
		models: { panel: PANEL_MODELS, judge: JUDGE_MODEL, synthesizer: SYNTHESIZER_MODEL },
	};

	if (usablePanel.length === 0) {
		progress.stage = "failed";
		progress.message = "all panel models failed";
		emit();
		return {
			text: `All fusion panel models failed.\n\n${formatPanelForPrompt(panel)}`,
			details,
			isError: true,
		};
	}

	progress.stage = "judge";
	progress.message = "judging panel outputs";
	progress.judge.status = "running";
	emit();
	const analysis = await runPiPrompt({
		name: "fusion-judge",
		model: JUDGE_MODEL,
		prompt: buildJudgePrompt(prompt, usablePanel),
		cwd,
		signal,
		thinking: THINKING_LEVEL,
	});
	details.analysis = analysis;
	progress.judge.status = analysis.exitCode === 0 && analysis.stopReason !== "error" ? "done" : "failed";
	progress.judge.durationMs = analysis.durationMs;
	progress.judge.usage = analysis.usage;
	emit(progress.judge.status === "done" ? "judge complete" : "judge failed");

	if (analysis.exitCode !== 0 || analysis.stopReason === "error") {
		progress.stage = "failed";
		progress.message = "judge failed";
		emit();
		return {
			text: `Fusion judge failed. Panel outputs are below.\n\n${formatPanelForPrompt(usablePanel)}`,
			details,
			isError: true,
		};
	}

	progress.stage = "synthesizer";
	progress.message = "synthesizing final answer";
	progress.synthesizer.status = "running";
	emit();
	const final = await runPiPrompt({
		name: "fusion-synthesizer",
		model: SYNTHESIZER_MODEL,
		prompt: buildSynthPrompt(prompt, usablePanel, analysis.output),
		cwd,
		signal,
		thinking: THINKING_LEVEL,
	});
	details.final = final;
	progress.synthesizer.status = final.exitCode === 0 && final.stopReason !== "error" ? "done" : "failed";
	progress.synthesizer.durationMs = final.durationMs;
	progress.synthesizer.usage = final.usage;
	emit(progress.synthesizer.status === "done" ? "synthesis complete" : "synthesis failed");

	if (final.exitCode !== 0 || final.stopReason === "error") {
		progress.stage = "failed";
		progress.message = "synthesizer failed";
		emit();
		return {
			text: `Fusion synthesizer failed. Judge analysis is below.\n\n${analysis.output}`,
			details,
			isError: true,
		};
	}

	progress.stage = "done";
	progress.message = "complete";
	emit();
	return { text: final.output, details };
}

const FusionParams = Type.Object({
	prompt: Type.String({ description: "The task/question to send to the OSS Fusion panel." }),
	mode: Type.Optional(
		Type.Union([Type.Literal("readonly"), Type.Literal("full"), Type.Literal("sandbox"), Type.Literal("patch-chain")], {
			description: "Tool/safety mode. readonly inspects only, full can edit the real tree, sandbox creates independent temporary copies, patch-chain sequentially refines one temporary copy. Default: readonly.",
		}),
	),
});

function parseFusionCommandArgs(args: string): { mode: FusionMode; prompt: string } {
	let mode = DEFAULT_FUSION_MODE;
	const parts = args.trim().split(/\s+/).filter(Boolean);
	const promptParts: string[] = [];
	for (let i = 0; i < parts.length; i++) {
		const part = parts[i];
		if (part === "--readonly") {
			mode = "readonly";
			continue;
		}
		if (part === "--full") {
			mode = "full";
			continue;
		}
		if (part === "--sandbox") {
			mode = "sandbox";
			continue;
		}
		if (part === "--patch-chain" || part === "--patchchain") {
			mode = "patch-chain";
			continue;
		}
		if (part === "--mode" && parts[i + 1]) {
			mode = parseFusionMode(parts[++i]) ?? mode;
			continue;
		}
		promptParts.push(part);
	}
	return { mode, prompt: promptParts.join(" ").trim() };
}

export default function ossFusionExtension(pi: ExtensionAPI) {
	pi.registerMessageRenderer("oss-fusion", (message, { expanded }, theme) => {
		const details = message.details as FusionDetails | undefined;
		const mdTheme = getMarkdownTheme();
		const container = new Container();
		container.addChild(new Text(theme.fg("toolTitle", theme.bold("OSS Fusion")), 0, 0));
		if (details) container.addChild(new Text(theme.fg("dim", summarizeDetails(details)), 0, 0));
		container.addChild(new Spacer(1));
		container.addChild(new Markdown(String(message.content ?? ""), 0, 0, mdTheme));

		if (expanded && details?.analysis?.output) {
			container.addChild(new Spacer(1));
			container.addChild(new Text(theme.fg("muted", "─── Judge analysis ───"), 0, 0));
			container.addChild(new Markdown(details.analysis.output, 0, 0, mdTheme));
		}
		return container;
	});

	pi.registerTool({
		name: "oss_fusion",
		label: "OSS Fusion",
		description:
			"Run a local/open-source model panel, judge the responses, and synthesize a final answer. Useful for hard research, architecture, debugging, and review questions.",
		promptSnippet: "Fan a hard prompt out to MiniMax M3, Fireworks Kimi K2.7 Fast, and llama.cpp Qwen, then judge and synthesize.",
		promptGuidelines: [
			"Use oss_fusion for complex questions where independent model diversity and synthesis are likely to improve reliability.",
			"Do not use oss_fusion for trivial edits or simple file reads; it is slower and runs multiple model calls.",
			"Use oss_fusion mode=readonly for investigation/review, mode=sandbox for independent implementation experiments, mode=patch-chain for sequential patch refinement in one temporary copy, and mode=full only when the user explicitly wants panel agents to edit the real working tree.",
		],
		parameters: FusionParams,
		async execute(_toolCallId, params, signal, onUpdate, ctx) {
			try {
				const mode = parseFusionMode(params.mode) ?? DEFAULT_FUSION_MODE;
				const result = await runFusion(params.prompt, ctx.cwd, mode, signal, (progress) => {
					const lines = formatProgressLines(progress);
					onUpdate?.({ content: [{ type: "text", text: lines.join("\n") }] });
					if (ctx.hasUI) {
						ctx.ui.setStatus("oss-fusion", compactProgressStatus(progress));
						ctx.ui.setWidget("oss-fusion", lines, { placement: "belowEditor" });
					}
				});
				return {
					content: [{ type: "text", text: result.text }],
					details: result.details,
					isError: result.isError,
				};
			} finally {
				if (ctx.hasUI) {
					ctx.ui.setStatus("oss-fusion", "");
					ctx.ui.setWidget("oss-fusion", undefined);
				}
			}
		},
	});

	pi.registerCommand("fusion", {
		description: "Run OSS Fusion. Usage: /fusion [--readonly|--sandbox|--patch-chain|--full] <prompt>",
		handler: async (args, ctx) => {
			const { mode, prompt } = parseFusionCommandArgs(args);
			if (!prompt) {
				ctx.ui.notify("Usage: /fusion [--readonly|--sandbox|--patch-chain|--full] <question or task>", "warning");
				return;
			}

			try {
				const result = await runFusion(prompt, ctx.cwd, mode, undefined, (progress) => {
					const lines = formatProgressLines(progress);
					ctx.ui.setStatus("oss-fusion", compactProgressStatus(progress));
					ctx.ui.setWidget("oss-fusion", lines, { placement: "belowEditor" });
				});
				pi.sendMessage({
					customType: "oss-fusion",
					content: result.text,
					display: true,
					details: result.details,
				});
			} catch (error) {
				ctx.ui.notify(error instanceof Error ? error.message : String(error), "error");
			} finally {
				ctx.ui.setStatus("oss-fusion", "");
				ctx.ui.setWidget("oss-fusion", undefined);
			}
		},
	});
}
