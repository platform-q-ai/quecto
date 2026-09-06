# Bound candidate v1/v2 launch — source-reviewed, no agents launched

Source changes are reserved for **only voted accepted guidance supplied by parent**.
No source change until the approved vote/candidate payload arrives. Existing frozen
helpers/rubric/catalog remain untouched. Parent owns panel retrieval.

## Practical answer to the pre-provisioning nudge concern

Use **taskless bound spawn → parent provision → exact product prompt**, with no
extra system instruction and no custom container config required. Source does NOT
run an idle-nudge loop on startup or on inspection queries:

- `src/interface/tool_runtime.rs:368–416` selects/binds the spec during registration.
- `src/domain/workflow/engine.rs:332–341` makes an active bound template eligible for
  auto-continue. **workflow:false alone does not suppress active-step nudges.**
- However `src/interface/cli/uds_multi.rs:303–313,347–370` enters a dispatch loop that
  waits for a command/notification; startup does not call `drain_pending_and_nudge`.
- `src/interface/cli/uds.rs:381–393` invokes that drain AFTER a prompt execution.
  `uds_dispatch.rs:325–366` can also invoke it for steer/follow-up; `uds_multi.rs:412–417`
  for a child's retained notification. A fresh taskless child has no descendants and
  disabled spawn tools. Do not send setup/acknowledgment prompts, steer or follow-up.
- `spawn_launch_args.rs:164–173` forwards spec independently of workflow flag;
  `spawn_entry.rs:77` marks taskless entries idle. Model prompt submission requires
  a supplied task in the application launch path. Read-only state/history inspection
  does not drain workflow nudges (`uds_query.rs`).

Thus binding is ready before task, but does not itself start a turn. One pre-prompt
state/history inspection is a prudent boundary check, not polling. If unexpected
model activity is found, record it and stop that run; do not retroactively count it
as clean. This is source-supported, not claimed newly live-tested. A custom adapter
that copies before child launch would be a fallback only if runtime contradicts the
source; no reason to build it or inject behavioral system coaching now.

## Commands (parent, once candidate is approved)

```sh
# Make exact immutable candidate/spec/prompt + fixture; no runtime calls.
python3 -B workflow-evaluation/prepare_revised_run.py prepare \
  --task feature-v1 --candidate /absolute/approved-feature.json \
  --run-dir workflow-evaluation/runs/feature-v1-001
```

Submit that run's `spawn-request.json` verbatim to spawn: fresh named sandbox,
`openai-oauth/gpt-6-astra`, low effort, workflow false, guards false, FULL
`workflow_spec.template`, and **no task**. Save UUID/ref/workspace. Then:

```sh
python3 -B workflow-evaluation/prepare_revised_run.py provision \
  --run-dir workflow-evaluation/runs/feature-v1-001 \
  --workspace <spawn-returned-workspace>
```

This copies fixture bytes only into `/workspace/task`, verifies hashes, and writes
capture-compatible `provision.json` with candidate/spec hashes. It does NOT call
frozen `provision.py` or emit a built-in selection preamble. Parent sends the exact
run `task.txt` via `agent_cmd prompt` to that idle UUID after verification. Prompt is
catalog product body only. For v2 use `feature-v2` and the SAME unchanged approved
candidate; for a changed candidate use base again per frozen streak rules. The helper
supports all 15 catalog task IDs, not only feature. Parent retains its decision on
unchanged qualifying baselines versus new candidates; helper does not infer votes.

After completion: usual bare report, paginated history and captured artifacts, then
`capture_and_verify.py RUN_DIR`; it will use retained `assigned-template.json`, not
an inferred built-in. Parent cleans up only after reopened evidence. Never reuse a
worker/container across variants. Existing launch-mode/blinding/metric limits apply.

## Focused Cargo feasibility

Source-validation-plan.md filters target real source tests. Workspace package is
`quecto-agentic-harness`, library is `quecto`, test targets exist. Host cargo/rustc
both report **1.97.1**; target/debug artifacts exist but freshness is unknown. No
Cargo build/test run was performed. Use `cargo test -p quecto-agentic-harness --lib
built_in_default_templates_` first when authorized, then domain/spec filters and
single integration test from the plan. `--lib` avoids building unrelated workspace
packages/tests but still resolves/builds necessary dependencies. Do not treat cached
artifact presence or a zero-selected-test run as validation. No broad workspace build
is needed for guidance-only changes.

## Local helper validation

Ten prepare-only cases (five workflows × v1/v2) used unchanged source snapshots as
syntax/shape examples, NOT approved revisions. Checked exact candidate/spec object
agreement, task omitted from spawn, and prompt without selection preamble. All passed
in temporary local directories; zero agents/container calls. Provision stage uses
already-tested parent shell/copy primitives, not separately live-exercised here.
Frozen manifest hashes verified unchanged. Candidate size/shape checks complement,
not replace, later Rust domain validation and compiled source-object equality.
