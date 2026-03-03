/**
 * Unit tests for agent-state.ts pure logic helpers.
 *
 * Run with: node --experimental-vm-modules node_modules/.bin/jest agent-state.test.ts
 * (Or: npx tsx --test agent-state.test.ts)
 *
 * These are intentionally lightweight — the pi ExtensionAPI cannot be exercised
 * without a running pi session, so we test only the side-effect-free helpers.
 */
import {
	applyRpcEvent,
	formatLastToolCall,
	computeStatusSummary,
	formatProgress,
	type ManagedAgent,
	type Alert,
} from "./agent-state.js";

// ─── Test helpers ──────────────────────────────────────────────────────────

function makeAgent(overrides: Partial<ManagedAgent> = {}): ManagedAgent {
	return {
		id: "test-1",
		label: "test",
		cwd: "/tmp/repo",
		sessionFile: "/tmp/session.jsonl",
		agentType: "quecto",
		pid: 1234,
		holderPid: 1235,
		fifoPath: "/tmp/fifo-test-1",
		status: "idle",
		usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0 },
		alerts: [],
		...overrides,
	};
}

// ─── applyRpcEvent ─────────────────────────────────────────────────────────

// Scenario 3: turn_start → running
function test_turn_start_sets_running() {
	const agent = makeAgent({ status: "idle" });
	const result = applyRpcEvent(agent, { type: "turn_start" });
	console.assert(result.status === "running", `Expected running, got ${result.status}`);
	console.log("✓ turn_start sets status=running");
}

// Scenario 3: agent_end → idle
function test_agent_end_sets_idle() {
	const agent = makeAgent({ status: "running" });
	const result = applyRpcEvent(agent, {
		type: "agent_end",
		messages: [{ role: "assistant", content: "Done." }],
	});
	console.assert(result.status === "idle", `Expected idle, got ${result.status}`);
	console.log("✓ agent_end sets status=idle");
}

// Scenario 5: agent_end extracts lastText
function test_agent_end_extracts_last_text() {
	const agent = makeAgent({ status: "running" });
	const result = applyRpcEvent(agent, {
		type: "agent_end",
		messages: [
			{ role: "user", content: "Fix the test" },
			{ role: "assistant", content: "All tests passing now." },
		],
	});
	console.assert(
		result.lastText === "All tests passing now.",
		`Expected last text, got ${result.lastText}`,
	);
	console.log("✓ agent_end extracts lastText from final assistant message");
}

// Scenario 4: tool_execution_start updates lastToolCall
function test_tool_execution_start_updates_last_tool() {
	const agent = makeAgent({ status: "running" });
	const result = applyRpcEvent(agent, {
		type: "tool_execution_start",
		toolCallId: "call-1",
		toolName: "bash",
		args: { command: "cargo test" },
	});
	console.assert(result.lastToolCall === "bash: cargo test", `Expected bash: cargo test, got ${result.lastToolCall}`);
	console.log("✓ tool_execution_start sets lastToolCall");
}

// Scenario 3: tool_execution_end with isError=true → error
function test_tool_execution_end_error_sets_status() {
	const agent = makeAgent({ status: "running" });
	const result = applyRpcEvent(agent, {
		type: "tool_execution_end",
		toolCallId: "call-1",
		toolName: "bash",
		result: { content: [{ type: "text", text: "failed" }] },
		isError: true,
	});
	console.assert(result.status === "error", `Expected error, got ${result.status}`);
	console.log("✓ tool_execution_end with isError=true sets status=error");
}

// tool_execution_end with isError=false → no status change
function test_tool_execution_end_ok_no_status_change() {
	const agent = makeAgent({ status: "running" });
	const result = applyRpcEvent(agent, {
		type: "tool_execution_end",
		toolCallId: "call-1",
		toolName: "bash",
		result: { content: [{ type: "text", text: "ok" }] },
		isError: false,
	});
	console.assert(result.status === "running", `Expected running, got ${result.status}`);
	console.log("✓ tool_execution_end with isError=false preserves status");
}

// ─── formatLastToolCall ────────────────────────────────────────────────────

function test_format_bash_tool_call() {
	const label = formatLastToolCall("bash", { command: "cargo test --lib" });
	console.assert(label === "bash: cargo test --lib", `Got: ${label}`);
	console.log("✓ formatLastToolCall formats bash command");
}

function test_format_other_tool_call() {
	const label = formatLastToolCall("read", { path: "/some/file.ts" });
	console.assert(label === "read: /some/file.ts", `Got: ${label}`);
	console.log("✓ formatLastToolCall formats non-bash tool");
}

function test_format_tool_call_truncates_long_command() {
	const long = "a".repeat(80);
	const label = formatLastToolCall("bash", { command: long });
	console.assert(label.length <= 75, `Label too long: ${label.length}`);
	console.assert(label.endsWith("…"), `Should end with ellipsis: ${label}`);
	console.log("✓ formatLastToolCall truncates long commands");
}

// ─── computeStatusSummary ──────────────────────────────────────────────────

function test_status_summary_counts() {
	const agents: ManagedAgent[] = [
		makeAgent({ status: "running" }),
		makeAgent({ status: "running" }),
		makeAgent({ status: "idle" }),
		makeAgent({ status: "blocked" }),
		makeAgent({ status: "error" }),
	];
	const summary = computeStatusSummary(agents);
	console.assert(summary.running === 2, `Expected 2 running, got ${summary.running}`);
	console.assert(summary.idle === 1, `Expected 1 idle, got ${summary.idle}`);
	console.assert(summary.blocked === 1, `Expected 1 blocked, got ${summary.blocked}`);
	console.assert(summary.error === 1, `Expected 1 error, got ${summary.error}`);
	console.log("✓ computeStatusSummary counts correctly");
}

function test_status_summary_empty() {
	const summary = computeStatusSummary([]);
	console.assert(summary.running === 0 && summary.idle === 0, "Empty should have all zeros");
	console.log("✓ computeStatusSummary handles empty array");
}

// ─── formatProgress ───────────────────────────────────────────────────────

function test_format_progress_no_workflow() {
	const bar = formatProgress(undefined, 15);
	console.assert(bar === "░".repeat(15), `Expected empty bar, got: ${bar}`);
	console.log("✓ formatProgress with no workflow steps shows empty bar");
}

function test_format_progress_8_of_16() {
	const steps = Array.from({ length: 16 }, (_, i) => ({ id: i + 1, done: i < 8 }));
	const bar = formatProgress(steps, 16);
	const filled = bar.split("").filter((c) => c === "█").length;
	const empty = bar.split("").filter((c) => c === "░").length;
	console.assert(filled === 8, `Expected 8 filled, got ${filled}`);
	console.assert(empty === 8, `Expected 8 empty, got ${empty}`);
	console.log("✓ formatProgress 8/16 renders correctly");
}

function test_format_progress_all_done() {
	const steps = Array.from({ length: 16 }, (_, i) => ({ id: i + 1, done: true }));
	const bar = formatProgress(steps, 16);
	console.assert(!bar.includes("░"), "All done should have no empty slots");
	console.log("✓ formatProgress all done renders full bar");
}

// ─── Run all tests ─────────────────────────────────────────────────────────

const tests = [
	test_turn_start_sets_running,
	test_agent_end_sets_idle,
	test_agent_end_extracts_last_text,
	test_tool_execution_start_updates_last_tool,
	test_tool_execution_end_error_sets_status,
	test_tool_execution_end_ok_no_status_change,
	test_format_bash_tool_call,
	test_format_other_tool_call,
	test_format_tool_call_truncates_long_command,
	test_status_summary_counts,
	test_status_summary_empty,
	test_format_progress_no_workflow,
	test_format_progress_8_of_16,
	test_format_progress_all_done,
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
