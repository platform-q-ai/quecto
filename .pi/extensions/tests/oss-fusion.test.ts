const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");

const extensionPath = join(process.cwd(), ".pi/extensions/oss-fusion.ts");
const source = readFileSync(extensionPath, "utf8");

function assertContains(text, expected) {
	assert.ok(text.includes(expected), `expected oss-fusion.ts to contain ${JSON.stringify(expected)}`);
}

assertContains(source, "pi.registerTool({");
assertContains(source, 'name: "oss_fusion"');
assertContains(source, 'pi.registerCommand("fusion"');
assertContains(source, 'pi.registerMessageRenderer("oss-fusion"');

for (const mode of ["readonly", "full", "sandbox", "patch-chain"]) {
	assertContains(source, `"${mode}"`);
}

for (const envName of [
	"OSS_FUSION_MODELS",
	"OSS_FUSION_JUDGE_MODEL",
	"OSS_FUSION_SYNTHESIZER_MODEL",
	"OSS_FUSION_MODE",
	"OSS_FUSION_MAX_PANEL_MODELS",
	"OSS_FUSION_CONCURRENCY",
	"OSS_FUSION_ALLOW_TOOL_MUTATION",
	"OSS_FUSION_ALLOW_SCRATCH_MUTATION",
]) {
	assertContains(source, envName);
}

assertContains(source, 'ctx.ui.setWidget("oss-fusion"');
assertContains(source, 'ctx.ui.setStatus("oss-fusion"');
assertContains(source, "createSandbox");
assertContains(source, "collectSandboxDiff");
assertContains(source, "mapWithConcurrency");
assertContains(source, "buildChildEnv");
assertContains(source, '"--no-extensions"');
assertContains(source, "StringEnum([\"readonly\", \"full\", \"sandbox\", \"patch-chain\"]");
assertContains(source, "redactSecrets");
assertContains(source, "appendBoundedTrace");
assertContains(source, "SANDBOX_EXCLUDE_GLOBS");
assertContains(source, "if (stat.isSymbolicLink()) return true");
assertContains(source, '"--no-dereference"');

console.log("✓ oss-fusion extension is installed with tool, command, renderer, bounded execution, and UI progress");
