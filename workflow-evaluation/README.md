# Current status: runnable pilot infrastructure ready

**Use [LAUNCH.md](LAUNCH.md) as the current executable recipe.** It supersedes the
initial proposal's infrastructure prerequisites below. Frozen rubric/task fairness
fixes incorporate [design-review.md](design-review.md). One authorized preparation
agent was launched and cleaned up; **zero scored workers or judges** were launched.
Default model verified: `openai-oauth/gpt-6-astra`, effort low; named `sandbox`,
rootless Podman, Python 3.13.5. Idle spawn → parent provision → exact prompt → paginated
history archive → artifacts → cleanup. Task/rubric freeze: `freeze-manifest.json`.

The text below is the retained initial design, not a requirement to build unsupported
ideal controls. In particular no enforced 40k total-token cap, isolated auth home,
complete live-event stream or pinned custom image is required to launch. Unknown
metrics are recorded as unavailable. Revised payload files intentionally omit task
and set workflow false to suppress premature selector nudges; selection remains in
the later task prompt and the workflow tool remains available.

---

# Workflow quality evaluation — proposed design, not execution

Scope owner: **experiment design and implementation-source discovery only**.
Branch `chore/workflow-quality`, inspected source commit
`c7f6ac7d871bd1b7ddaa2d154900a9fddf811ead`.

**No workers, judges, containers, baseline experiments, or workflow revision runs
were launched. No scores, votes, or qualifying runs are claimed. No workflow source
or goal.md edits are part of this proposal.** Local fixture-authoring self-checks are
recorded separately and must never be counted as experiment results.

## Contents

* [SOURCES.md](SOURCES.md): actual built-in source paths, runtime precedence, exact
  resolved/directory JSON distinctions, spec binding, bounds and guard semantics.
* [TASKS.md](TASKS.md): **five small baseline tasks plus ten varied validation tasks**,
  verbatim spawn-ready product prompts, acceptance cards and frozen oracle bodies.
* `catalog.py`: deterministic UTF-8 fixture bytes, prompts, allowed paths, oracles and
  task-specific critical failures. Evaluator-only; no install dependencies.
* `fixture_tool.py`: refusal-safe fresh setup, snapshots and independent verification
  with stdout/stderr/exit codes. No agent/container launcher. Requires Python 3.10+.
* [RUBRIC.md](RUBRIC.md): fixed evidence-based **100 points**, critical-failure gate,
  independent three-member scoring panel, proposal/voting rules and consistency rule.
* `templates/builtin-*.json`: complete by-value baseline templates extracted from
  actual source tuples, for evidence/equality checks, not source modifications.
* `resolved-template.schema.json`: derived schema aid, **not** an upstream formal
  schema and not the strict directory-file format.
* `spawn-requests/*.json`: baseline call payload proposals, one per workflow, with
  low effort and fresh-container request. They are **not submitted**, and cannot
  be executed faithfully before the provisioning/model preflight below.
* `records/*.json`: blank evidence/ballot/candidate examples, no invented results.
* `design_selfcheck.py`, `design-selfcheck.json`: fixture authoring validation only.
* `design-manifest.json`: file digests for freezing/review. If proposal edits are
  accepted, regenerate manifest and freeze the final version before experiments.

## Workload choices and portability

Each baseline contains one small module or CLI, a few inputs, and at most a handful
of existing smoke checks. No network, package manager, database, compilation, Git
repository, remote account, randomness, timing behavior, or live services are
required. Python standard library was chosen for an inexpensive, deterministic
fresh-container floor, **not** to introduce Python-specific workflow guidance.
Investigations include log/source analysis and ambiguous evidence; chores include
non-executable prose/config/link checks; refactors require actual structural changes
as well as parity. This allows evidence-first alternatives rather than rewarding
TDD theater. Larger-system and cross-language transfer remain untested limitations.

All workers see only their isolated fixture and product requirement. They do not
see other task fixtures, goal.md, this evaluation directory, expected solutions,
oracles, rubric, judge feedback or candidate rationale. Sharing this checkout as a
worker workspace would leak the benchmark; do not do that. No worker designs its own
fixture. Worker adds tests to the fixture, not evaluator oracles.

## Proposed controlled experiment

### Preflight before anyone launches a worker

1. Re-read `goal.md`; accept/freeze tasks, rubric, gate, budgets and panel protocol.
   Hash the source revision, template JSON, fixture, task body, all worker-visible
   instructions and configuration. Preserve unrelated checkout work.
2. Select and record one exact supported worker model/provider and `effort: low`.
   Do not change model, tool capabilities, system instructions, context limit, token
   budget or environment between a workflow's baseline/revision/validation runs.
   Proposed ceiling: **20 minutes and 40,000 total reported input+output tokens per
   run**, first limit reached; record provider cached-token accounting separately.
   If the harness cannot enforce or measure this definition, resolve that before
   preregistration rather than silently adopting a different limit. No child workers,
   panel steering or live rescue. Use identical available worker tools; disable
   spawning/delegation and network tools in the trusted run configuration if supported.
3. Pin a fresh container image **by digest** with Python 3.10+ and the same agent
   binary/config. Record `python3 --version`, OS/image digest, locale, timezone,
   working directory, executable hash/version, tool catalog and environment variable
   names/allowlisted nonsecret values. Set `PYTHONDONTWRITEBYTECODE=1`,
   `PYTHONHASHSEED=0`, and `TZ=UTC` for all runs. No secrets or external-service
   credentials inside fixture/runtime apart from separately isolated provider access.
4. Provision exactly one chosen fixture at `/workspace/task` **before the model's
   first token**. `fixture_tool.py setup` is the deterministic recipe. Use a trusted
   image-build/prelaunch copy boundary, not a task instruction to generate fixture
   files and not an LLM setup worker. The exported worker payload contains only
   `TASKS[id]['files']`. Keep evaluator files outside worker-visible mounts.
   Each run gets a brand-new container, not a cleaned/reused container. Archive the
   pristine fixture and file hashes outside it.
5. Ensure baseline uses the actual built-in library: no configured `workflow.dir`,
   repo/home `.quecto/workflows`, custom inline `workflow.templates`, or selector
   prompt that changes behavior. Hold `auto_continue:true`, `completion_nudge:true`,
   `workflow:true`, `workflow_guards:false`. Verify compiled source matches the
   archived built-in snapshot; record resolved configuration and observed workflow
   events/guidance. IDs alone are insufficient. A source/binary mismatch blocks a
   claimed source baseline.
6. The generated spawn payloads use `container:true` (fresh default container) and
   a fixed product prompt plus **only** the required baseline selection sentence.
   Pin model and preparation in the trusted run configuration or add the exact
   preregistered `model` field to every payload. If default container preparation
   cannot install the fixture before activation, use a named per-task prebuilt
   configuration with `container:{"mode":"new","container_config":"<pinned-name>"}`.
   Do not pass a repo field, share a container, or pretend an unconfigured `true`
   request already supplies these fixtures. Record this concrete provisioning binding
   in `run.json`. **That infrastructure choice is still pending; not tested here.**

The task *bodies* are ready now. Payloads are ready to finalize once model and trusted
fixture provisioning are bound. There is deliberately no code here that starts work.

### Runs and comparisons (later execution owner)

* Baseline: one fresh low-effort worker per workflow on `*-base`. Assignment preamble:
  `Select the built-in <id> workflow for this task.` The body only describes the
  software request. No hints about testing sequence, hypotheses, workflow boxes,
  evaluation score or improvements. Template selection is the single required
  assignment instruction, not an attempt to coach behavior.
* Observe the actual run without progress nudges from the experimenter. Normal
  harness handoffs are part of the treatment. Record parent messages, even accidental
  interventions. Do not treat a completion notification as the worker's answer:
  retrieve the completed child's unread report with its spawn-returned UUID and
  archive the full transcript through the supported transcript/export mechanism.
  `get_messages` alone may not contain all tool events; missing events invalidate
  behavioral claims. Avoid polling/wait loops; occasional supervision only.
* Score and vote as in RUBRIC.md, blind to revision labels. Freeze accepted candidate
  JSON and hash before spawning the next worker. No task prompt changes to compensate
  for workflow defects. Change one coherent set of voted guidance edits at a time.
* Revised paired run: same baseline fixture/task, fresh container/agent, same effort,
  model, budgets and config, but bind the **full candidate object** through
  `workflow_spec:{"template": candidate}`. Omit only the baseline selection preamble
  because the assigned template starts active. Treat that launch-mode difference as
  a confound, not hidden proof that text alone caused improvement. An optional
  unchanged-template bound control can isolate it; report separately, not as a
  substitute for the requested baseline.
* At first qualifying score, later source owner updates the actual template source
  on this branch, checks source/runtime equality, and proceeds to frozen `v1`, `v2`
  variants for the three-run rule. No source edits in this design assignment.
  Streak/reset handling is fixed in RUBRIC.md; failures and infrastructure-invalid
  attempts must remain in the run ledger.

## Evidence collection contract

Evaluator retains **append-only, content-addressed evidence outside the worker**:

```
runs/<anonymous-run-id>/
  run.json                    # manifest: see records/run.example.json
  task.txt                    # exact prompt including assignment if present
  assigned-template.json      # exact resolved content, even for built-in baseline
  fixture-before/             # pristine worker-visible tree
  before-files.json            # bytes, paths; record modes/symlinks separately
  events.raw.jsonl             # full ordered messages, tool args/results/errors
  transcript-index.json        # stable event IDs/offsets for panel citations
  workflow-events.jsonl        # select/check/skip/uncheck/reset, nudges, snapshots
  parent-interventions.jsonl   # empty is explicit; never omit rescue messages
  fixture-after/              # final tree, including added tests/artifacts
  final-files.json
  diff.patch-or-file-diff.txt  # no Git dependency; archive added/deleted file bytes
  worker-final.txt
  session-stats.json           # tokens, cost if available, elapsed, completion reason
  verification.json           # evaluator oracle + worker suite outputs on archived copy
  replay/                     # selected original/final checks, with exact commands
  judges/judge-1.initial.json  # all three original and final ballots preserved
  judges/judge-1.final.json
  panel-summary.json          # medians, gate, disputes; no fabricated unavailable data
  evidence-sha256.json
```

Capture tool start/end order, failures, exit codes, cwd/environment, edits as they
occur and workflow events. Take parent-controlled snapshots at first edit, before
claimed RED/GREEN checks where supported, and final state; a timestamped transcript
plus patch sequence may reconstruct intermediate bytes, but explicitly note gaps.
For meaningful red-to-green proof, replay added regression tests against both the
original and final code in disposable evaluator sandboxes, recording commands and
causal failure. For refactors, replay characterization on both versions. For docs,
execute the actual documented command or resolve links and compare preserved prose;
for investigation, compare cited lines and counterfactual probe results to the final
claim. Do not run untrusted worker tests on the parent's checkout or where they can
edit the retained oracle/evidence. `fixture_tool.py` stages a copy but **does not
provide OS isolation**; use an external disposable sandbox with no secrets/network.

The verifier records file preservation and frozen oracle outcomes but does not
assign scores. Judges must inspect shared-helper/module extraction, guard-clause
structure, tests that might be vacuous, report reasoning and no-edit constraints.
Hashes prove bytes at capture time, not absence of transient writes: use tool events
and filesystem auditing when available, and disclose that absence of such auditing
limits read-only guarantees. Record empty directories/modes separately if relevant;
current helper snapshots files/symlinks, not every filesystem metadata attribute.

After completion, obtain unread worker report (not passive status text), export full
events, copy/snapshot files, replay verification, hash evidence, verify it can be
reopened **outside the container**, then remove the container. Keep container
reference/UUID, cleanup timestamp and artifact location in the ledger. Do not let
cleanup destroy the only transcript or intermediate failing test.

### Minimum analysis/report

Per workflow show baseline raw panel/individual scores and gate, every candidate's
votes/diff/hash, task/attempt/model/effort/seed if available, revised score and gate,
verification outcomes, infrastructure-invalid attempts, running streak and final
status. Report paired changes descriptively, not causal certainty from one run.
Keep cost, token/time and intervention counts as secondary outcomes (no rubric
weight changes). Store `not available` rather than inventing unavailable metrics.

## Design validation performed here

`python3 -B workflow-evaluation/design_selfcheck.py` successfully checked all 15
fixture sets in temporary local directories, with no workers or containers:

* every generated Python source and oracle compiles;
* deterministic setup is unchanged by evaluator staging;
* all supplied smoke suites pass initially;
* bug/feature/chore acceptance oracles fail on the intentionally incomplete original;
* read-only oracle observations and behavior-only refactor parity checks pass
  initially (expected: refactors must preserve behavior, not fabricate a failing
  test); module extraction also has an initially missing-module acceptance check.

Full initial stdout/stderr and catalog hashes are in `design-selfcheck.json`.
These checks establish fixture plausibility and plumbing, **not** end-to-end worker
or panel validity. Positive patched solutions, actual image provisioning, transcript
export completeness, runtime/source equality and low-effort workload calibration
remain preflight/execution checks. No scores or success claims follow from these
local authoring checks.
