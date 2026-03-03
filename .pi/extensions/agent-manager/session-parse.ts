/**
 * Agent Manager — session JSONL file parser.
 *
 * Reads a pi/quecto session JSONL file line by line and reconstructs
 * workflow state (steps + active issue) from `toolResult` entries where
 * `toolName === "workflow"`.
 *
 * This mirrors the `reconstructState` logic in quecto-workflow.ts but is
 * decoupled from the pi session API so it can work on raw file content.
 */
import * as fs from "node:fs";
import * as readline from "node:readline";

// ─── Types ─────────────────────────────────────────────────────────────────

export interface WorkflowStep {
	id: number;
	done: boolean;
	label?: string;
}

export interface ActiveIssue {
	number: number;
	title: string;
}

export interface WorkflowState {
	steps: WorkflowStep[];
	activeIssue?: ActiveIssue;
}

// ─── parseWorkflowStateFromLines ──────────────────────────────────────────

/**
 * Parse workflow state from an array of JSONL lines (e.g., from a session file).
 *
 * Returns the LAST `workflow` tool result's state, or `null` if none found.
 * Malformed JSON lines are silently ignored.
 */
export function parseWorkflowStateFromLines(lines: string[]): WorkflowState | null {
	let latest: WorkflowState | null = null;

	for (const line of lines) {
		const trimmed = line.trim();
		if (!trimmed) continue;

		let entry: unknown;
		try {
			entry = JSON.parse(trimmed);
		} catch {
			continue; // skip malformed lines
		}

		if (!isObject(entry)) continue;
		if (entry.type !== "message") continue;

		const msg = entry.message;
		if (!isObject(msg)) continue;
		if (msg.role !== "toolResult") continue;
		if (msg.toolName !== "workflow") continue;

		const details = msg.details;
		if (!isObject(details)) continue;
		if (!Array.isArray(details.steps)) continue;

		const steps: WorkflowStep[] = details.steps.map((s: unknown) => {
			if (isObject(s)) {
				return {
					id: Number(s.id),
					done: Boolean(s.done),
					label: typeof s.label === "string" ? s.label : undefined,
				};
			}
			return { id: 0, done: false };
		});

		let activeIssue: ActiveIssue | undefined;
		if (isObject(details.activeIssue)) {
			const ai = details.activeIssue;
			if (typeof ai.number === "number" && typeof ai.title === "string") {
				activeIssue = { number: ai.number, title: ai.title };
			}
		}

		latest = { steps, activeIssue };
	}

	return latest;
}

// ─── parseWorkflowStateFromFile ───────────────────────────────────────────

/**
 * Read a session JSONL file asynchronously and parse workflow state.
 * Returns null if the file doesn't exist or has no workflow entries.
 */
export async function parseWorkflowStateFromFile(filePath: string): Promise<WorkflowState | null> {
	try {
		await fs.promises.access(filePath);
	} catch {
		return null;
	}

	const lines: string[] = [];
	const rl = readline.createInterface({
		input: fs.createReadStream(filePath, { encoding: "utf-8" }),
		crlfDelay: Infinity,
	});

	for await (const line of rl) {
		lines.push(line);
	}

	return parseWorkflowStateFromLines(lines);
}

// ─── scanSessionsDir ──────────────────────────────────────────────────────

/**
 * Scan `~/.pi/agent/sessions/` (or custom dir) for session files grouped by cwd.
 *
 * Returns a map from cwd → array of session files (most recent first).
 */
export async function scanSessionsDir(
	sessionsDir: string,
): Promise<Map<string, string[]>> {
	const result = new Map<string, string[]>();

	let entries: fs.Dirent[];
	try {
		entries = await fs.promises.readdir(sessionsDir, { withFileTypes: true });
	} catch {
		return result;
	}

	for (const entry of entries) {
		if (!entry.isFile()) continue;
		if (!entry.name.endsWith(".jsonl")) continue;

		const filePath = `${sessionsDir}/${entry.name}`;

		// Read first few lines to find cwd
		const handle = await fs.promises.open(filePath, "r");
		try {
			const buf = Buffer.alloc(4096);
			const { bytesRead } = await handle.read(buf, 0, buf.length, 0);
			const head = buf.subarray(0, bytesRead).toString("utf-8");
			const firstLine = head.split("\n")[0]?.trim();
			if (firstLine) {
				let parsed: unknown;
				try {
					parsed = JSON.parse(firstLine);
				} catch {
					continue;
				}
				if (isObject(parsed) && typeof parsed.cwd === "string") {
					const cwd = parsed.cwd;
					if (!result.has(cwd)) result.set(cwd, []);
					result.get(cwd)!.push(filePath);
				}
			}
		} finally {
			await handle.close();
		}
	}

	// Sort each group newest first (by filename — pi uses timestamp-based names)
	for (const [, files] of result) {
		files.sort((a, b) => b.localeCompare(a));
	}

	return result;
}

// ─── Helpers ──────────────────────────────────────────────────────────────

function isObject(v: unknown): v is Record<string, unknown> {
	return typeof v === "object" && v !== null && !Array.isArray(v);
}
