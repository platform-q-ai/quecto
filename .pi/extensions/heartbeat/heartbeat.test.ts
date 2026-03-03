/**
 * Unit tests for heartbeat pure logic helpers.
 *
 * Run with: npx tsx heartbeat.test.ts
 */
import {
	formatNextTick,
	shouldSkipTick,
	buildHeartbeatPrompt,
	DEFAULT_HEARTBEAT_PROMPT,
	DEFAULT_INTERVAL_MS,
} from "./index.js";

// ─── formatNextTick ────────────────────────────────────────────────────────

function test_format_next_tick_seconds_only() {
	const result = formatNextTick(42_000);
	console.assert(result === "42s", `Expected "42s", got "${result}"`);
	console.log(`✓ formatNextTick(42000) = "${result}"`);
}

function test_format_next_tick_minutes_and_seconds() {
	const result = formatNextTick(3 * 60_000 + 42_000);
	console.assert(result === "3m 42s", `Expected "3m 42s", got "${result}"`);
	console.log(`✓ formatNextTick(3m42s) = "${result}"`);
}

function test_format_next_tick_exact_minutes() {
	const result = formatNextTick(5 * 60_000);
	console.assert(result === "5m 0s", `Expected "5m 0s", got "${result}"`);
	console.log(`✓ formatNextTick(5m) = "${result}"`);
}

function test_format_next_tick_zero() {
	const result = formatNextTick(0);
	console.assert(result === "0s", `Expected "0s", got "${result}"`);
	console.log(`✓ formatNextTick(0) = "${result}"`);
}

// ─── shouldSkipTick ────────────────────────────────────────────────────────

// Scenario 17: skip if hasPendingMessages
function test_skip_tick_if_pending_messages() {
	const result = shouldSkipTick({ hasPendingMessages: true });
	console.assert(result === true, "Should skip when pending messages exist");
	console.log("✓ shouldSkipTick=true when hasPendingMessages");
}

function test_no_skip_when_idle() {
	const result = shouldSkipTick({ hasPendingMessages: false });
	console.assert(result === false, "Should not skip when idle");
	console.log("✓ shouldSkipTick=false when idle");
}

// ─── buildHeartbeatPrompt ─────────────────────────────────────────────────

function test_default_prompt_contains_key_phrases() {
	const prompt = buildHeartbeatPrompt(undefined);
	console.assert(prompt.includes("Heartbeat"), "Should include Heartbeat header");
	console.assert(prompt.includes("agent_manager"), "Should reference agent_manager tool");
	console.assert(prompt.includes("blocked"), "Should mention blocked agents");
	console.log("✓ default heartbeat prompt contains key phrases");
}

function test_custom_prompt_used_when_set() {
	const custom = "Custom check-in prompt.";
	const prompt = buildHeartbeatPrompt(custom);
	console.assert(prompt === custom, `Expected custom prompt, got: ${prompt}`);
	console.log("✓ custom prompt overrides default");
}

// ─── Constants ────────────────────────────────────────────────────────────

function test_default_interval_is_5_minutes() {
	console.assert(DEFAULT_INTERVAL_MS === 300_000, `Expected 300000, got ${DEFAULT_INTERVAL_MS}`);
	console.log("✓ DEFAULT_INTERVAL_MS is 5 minutes (300000ms)");
}

function test_default_prompt_is_string() {
	console.assert(typeof DEFAULT_HEARTBEAT_PROMPT === "string" && DEFAULT_HEARTBEAT_PROMPT.length > 0);
	console.log("✓ DEFAULT_HEARTBEAT_PROMPT is a non-empty string");
}

// ─── Run ──────────────────────────────────────────────────────────────────

const tests = [
	test_format_next_tick_seconds_only,
	test_format_next_tick_minutes_and_seconds,
	test_format_next_tick_exact_minutes,
	test_format_next_tick_zero,
	test_skip_tick_if_pending_messages,
	test_no_skip_when_idle,
	test_default_prompt_contains_key_phrases,
	test_custom_prompt_used_when_set,
	test_default_interval_is_5_minutes,
	test_default_prompt_is_string,
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
