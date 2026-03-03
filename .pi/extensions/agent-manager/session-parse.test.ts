/**
 * Unit tests for session-parse.ts pure logic.
 *
 * Run with: npx tsx session-parse.test.ts
 */
import { parseWorkflowStateFromLines } from "./session-parse.js";

// ─── Helpers ──────────────────────────────────────────────────────────────

function makeWorkflowEntry(steps: Array<{ id: number; done: boolean }>, activeIssue?: { number: number; title: string }) {
	return JSON.stringify({
		type: "message",
		message: {
			role: "toolResult",
			toolName: "workflow",
			details: { steps, activeIssue },
		},
	});
}

// ─── Tests ────────────────────────────────────────────────────────────────

// Scenario 6: parse workflow state from JSONL lines
function test_parse_empty_returns_null() {
	const result = parseWorkflowStateFromLines([]);
	console.assert(result === null, `Expected null, got ${JSON.stringify(result)}`);
	console.log("✓ parseWorkflowStateFromLines returns null for empty input");
}

function test_parse_single_entry() {
	const steps = Array.from({ length: 16 }, (_, i) => ({ id: i + 1, done: i < 3 }));
	const lines = [makeWorkflowEntry(steps)];
	const result = parseWorkflowStateFromLines(lines);
	console.assert(result !== null, "Should parse successfully");
	console.assert(result!.steps.filter((s) => s.done).length === 3, "Should have 3 done steps");
	console.log("✓ parseWorkflowStateFromLines parses single entry");
}

function test_parse_takes_last_entry() {
	const steps1 = Array.from({ length: 16 }, (_, i) => ({ id: i + 1, done: i < 3 }));
	const steps2 = Array.from({ length: 16 }, (_, i) => ({ id: i + 1, done: i < 8 }));
	const lines = [makeWorkflowEntry(steps1), makeWorkflowEntry(steps2)];
	const result = parseWorkflowStateFromLines(lines);
	console.assert(result!.steps.filter((s) => s.done).length === 8, "Should take last entry (8 done)");
	console.log("✓ parseWorkflowStateFromLines takes last workflow entry");
}

function test_parse_active_issue() {
	const steps = Array.from({ length: 16 }, (_, i) => ({ id: i + 1, done: false }));
	const activeIssue = { number: 233, title: "feat: RPC mode" };
	const lines = [makeWorkflowEntry(steps, activeIssue)];
	const result = parseWorkflowStateFromLines(lines);
	console.assert(result!.activeIssue?.number === 233, "Should parse activeIssue number");
	console.assert(result!.activeIssue?.title === "feat: RPC mode", "Should parse activeIssue title");
	console.log("✓ parseWorkflowStateFromLines parses activeIssue");
}

function test_parse_skips_non_workflow_entries() {
	const lines = [
		JSON.stringify({ type: "message", message: { role: "user", content: "hello" } }),
		JSON.stringify({ type: "message", message: { role: "toolResult", toolName: "bash", details: {} } }),
		JSON.stringify({ type: "custom", customType: "workflow-state", data: {} }),
	];
	const result = parseWorkflowStateFromLines(lines);
	console.assert(result === null, "Should ignore non-workflow tool results");
	console.log("✓ parseWorkflowStateFromLines ignores non-workflow entries");
}

function test_parse_handles_malformed_json() {
	const steps = Array.from({ length: 16 }, (_, i) => ({ id: i + 1, done: i < 5 }));
	const lines = [
		"not valid json{{{{",
		makeWorkflowEntry(steps),
		"also{broken",
	];
	const result = parseWorkflowStateFromLines(lines);
	console.assert(result!.steps.filter((s) => s.done).length === 5, "Should parse valid entries despite bad ones");
	console.log("✓ parseWorkflowStateFromLines handles malformed JSON lines gracefully");
}

// ─── Run ──────────────────────────────────────────────────────────────────

const tests = [
	test_parse_empty_returns_null,
	test_parse_single_entry,
	test_parse_takes_last_entry,
	test_parse_active_issue,
	test_parse_skips_non_workflow_entries,
	test_parse_handles_malformed_json,
];

let passed = 0;
let failed = 0;
for (const t of tests) {
	try {
		t();
		passed++;
	} catch (e) {
		console.error(`✗ ${t.name}: ${e}`);
		failed++;
	}
}
console.log(`\nResults: ${passed} passed, ${failed} failed`);
if (failed > 0) process.exit(1);
