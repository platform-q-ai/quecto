# Runnable baseline launch recipe — frozen pilot v1.0

Ready for the parent to use. **No scored workers launched.** One authorized preparation
agent established shell/copy/archival paths and was cleaned up. Read goal.md before
an experiment cycle. This file supersedes stricter infrastructure ideals in the
initial design; missing secondary metrics do not block this pilot.

## Exact supported model and container choice

**`openai-oauth/gpt-6-astra`, `effort: "low"`** matches the current supported config
default AND the preparation agent's observed get_state response. Python **3.13.5**.
Trusted config is inherited from the parent (`/home/swq/.quecto/config.json` in this
session); no config copy or credentials printed/stored in this directory.

Use **named `sandbox`**, not `container:true`: current default `quecto` clones a
repository. `sandbox` uses the same runtime/image without a clone, avoiding benchmark
leakage and Git dependence. Runtime actually selected is **rootless Podman**, not
Docker (the scripts prefer Podman when installed). Tested image ID:
`b1f87e8917502e1963979a0ed43fd7866427c961c26aac0d1576a17774b63662`.
Record actual image ID per run; keep `quecto-box:local` unchanged during the pilot.
No image rebuild/digest-pinning machinery is needed now.

## 1. Spawn one idle agent in a fresh sandbox

Use `spawn-requests/baseline-<workflow>.json` verbatim as the spawn tool arguments.
For example:

```json
{"agent_id":"baseline-bugfix","container":{"mode":"new","container_config":"sandbox"},"model":"openai-oauth/gpt-6-astra","effort":"low","workflow":false,"workflow_guards":false,"disable_tools":["spawn","agent_cmd","web_fetch","web_search"]}
```

**No `task` key**: nothing for the worker to solve before fixture provisioning.
Save returned UUID, environment ref (C2 etc.), and exact workspace path.

`workflow:false` deliberately omits the CLI `--workflow` selector nudge; it does NOT
pass `--no-workflow` or remove the workflow tool. The task prompt explicitly asks for
selection. This prevents an idle pre-provisioning agent from being nudged into work.
Source: `spawn_launch_args.rs:164–173`, `interface/cli/agent.rs:387–396`,
`interface/tool_runtime.rs:353–388`. Auto-continue/completion remain configured true;
guards remain false. Record this deviation from the original `workflow:true` proposal
in every baseline. Revised bound candidates use full `workflow_spec` at spawn, keep
`workflow:false`, and omit only the selection sentence when finally prompted.

## 2. Parent provisions exact fixture before task prompt

From this checkout, substitute the **actual returned workspace** and unique run ID:

```sh
python3 -B workflow-evaluation/provision.py \
  --task bugfix-base \
  --workspace /home/swq/.quecto/container-environments/env-RETURNED/workspace \
  --run-dir workflow-evaluation/runs/bugfix-base-001
```

The helper does NOT spawn or prompt. It creates only the chosen fixture in the
parent run archive, obtains nonsecret runtime handle/socket from the environment's
`container` and `children.jsonl` files, creates `/workspace` inside the idle container
as root, restores host-user ownership, copies files, and compares SHA-256 hashes
inside the container. It refuses an existing run directory or task destination.
Only after `fixture_verified:true` is returned may the parent send the task.

The exact tested primitives are:

```sh
ENV=/home/swq/.quecto/container-environments/env-RETURNED
CONTAINER=$(cat "$ENV/container")
# Runtime state files above are nonsecret; NEVER cat provider-env or credentials.
podman exec --user 0 "$CONTAINER" sh -c 'mkdir -p /workspace; chown 1000:1000 /workspace'
podman cp /absolute/run/fixture-before "$CONTAINER":/workspace/task
podman exec --workdir /workspace/task "$CONTAINER" python3 -B --version
# An interactive shell is also available if needed, but not necessary:
# podman exec -i --workdir /workspace/task "$CONTAINER" sh
```

`provision.py` obtains actual UID/GID rather than assuming 1000. The agent process
starts in its empty sandbox workspace, while the task explicitly says
`/workspace/task`; no CWD relocation mechanism is required. Source fixtures have no
checkout/evaluator content. Interpreter bytecode files are ignored by frozen scoring.

## 3. Prompt the same idle UUID

Use `agent_cmd` with `command:"prompt"`, spawn-returned UUID, and the exact contents
of `spawn-requests/baseline-<workflow>.prompt.txt` (also written to run `task.txt`).
This is the **first benchmark task message**, after parent provisioning. Do not add
workflow coaching or setup instructions. No scored worker has been sent this prompt
yet. The preparation agent merely tested path existence, not the benchmark behavior.

Do unrelated parent work; do not poll/sleep/wait-loop. At a completion notification,
call bare `get_messages` with UUID and **no count/before** to obtain the unread report.
Occasional get_state is permitted to supervise. Use a soft 20-minute review/abort
budget measured from prompt, not preparation. No hard 40k token cap is imposed:
current harness reports some usage but an enforced total cap was not established.
Retain actual completion reason and available statistics; unknowns are null with a
reason, never inferred costs/tokens. Fixed config max_tokens=8192 is a per-response
setting, not an invented total-run budget. Do not change settings between conditions.

## 4. Archive paginated get_messages and artifacts BEFORE kill

Bare get_messages is only a report. Tool-based history recipe:

```json
{"agent_id":"<UUID>","command":"get_messages","count":20}
```

Save the entire response as `page-0000.json`. If `data.hasMoreBefore` is true, request:

```json
{"agent_id":"<UUID>","command":"get_messages","count":20,"before":"<data.before>"}
```

Continue finite **backward pagination**, using each returned `before`, until
`hasMoreBefore:false`. This is not polling. Do not guess cursors or stop because one
page lacks tool messages. Pages are newest-first; messages within a page carry IDs
and ordinals. Retain originals, deduplicate IDs and order by ordinal for judges.
Explicit pages do not consume the unread report cursor.

For exact file archival without manually copying tool results, use the included
read-only client against the child's identity-mounted socket from `provision.json`:

```sh
python3 -B workflow-evaluation/archive_session.py \
  --socket /run/user/1000/quecto-agent-UUID.sock \
  --output workflow-evaluation/runs/bugfix-base-001/transcript
```

This makes the same **paginated get_messages** requests using the source-defined
4-byte big-endian UDS framing, correlates response IDs, saves every page, get_state,
get_session_stats and ordered messages, and never prompts/launches. Tested with page
size 2: five pages retrieved all nine preparation messages, including actual bash
arguments/results. Preserve bare-report tool response separately in parent evidence.

```sh
podman cp "$CONTAINER":/workspace/task /absolute/run/fixture-after
python3 -B workflow-evaluation/fixture_tool.py snapshot \
  --root /absolute/run/fixture-after --output /absolute/run/after-files.json
```

Compare archived before/after bytes and replay frozen verification. Worker code must
not execute on the host simply because the helper can run it: run checks in the
still-live task container (`podman exec --workdir /workspace/task ...`) or another
operator sandbox, capturing exit code/stdout/stderr to run files. Pass the evaluator
oracle via parent-owned stdin/argv at this post-worker point only; don't install the
catalog in worker workspace. For example, parent Python can load `TASKS[id]['oracle']`
and invoke `subprocess.run(['podman','exec','--workdir','/workspace/task',container,
'python3','-B','-c',oracle],capture_output=True,text=True)`, saving the result JSON.
Run `python3 -B -m unittest discover -s tests -v` when tests exist. Archive first so
verification-generated files are not confused with worker edits. Structural refactor
and documentation semantics require panel inspection, not oracle exit code alone.

Inspect pages for collapsed/truncated content. Smaller pages can help frame limits,
but cannot magically recover already-collapsed content. Preserve available diffs and
commands; explicitly identify missing evidence. Complete token streams, filesystem
auditing, and every workflow broadcast are not required ideal controls. Missing
material evidence prevents claims relying on it; missing secondary metrics does not
invalidate otherwise supported scoring. Tool calls to workflow and their results
are the practical progression record.

After reopening archive files outside the container and hashing them, use:
`agent_cmd {"agent_id":"*","command":"kill_container","ref":"<returned ref>"}`.
Record result and optional `podman container exists "$CONTAINER"` (exit 1 = absent).
Never kill before artifact/transcript export.

## Practical limits, not new blockers

* The adapter identity-mounts `$HOME/.quecto` for provider authentication. Therefore
  workspace isolation is real but **credential/evaluator-home isolation is not a hard
  security boundary**. Do not read/print/copy config wholesale, environment dumps,
  provider-env, credentials, or Docker/Podman full inspect output. Shared auth and
  network access are documented confounds; disabled web tools are not network isolation.
* No dedicated system-prompt/context hashing, seed control, live token recorder,
  hard cost cap or continuous filesystem audit was established. Record unavailable.
* Runtime/source equality is supported by source snapshots and observed selected
  guidance, not assumed solely from template IDs. Binary build provenance can be
  recorded if readily available; no Rust rebuild is required to start the pilot.
* Frozen tasks are small/scaffolded probes. Report success as **qualifying on the
  frozen small-task suite**, not proof of universal/world-class performance.
* An unsupported metric or desirable control is a disclosed limitation, not a new
  hidden acceptance criterion. Preserve enough direct evidence to judge actual work.

## Preparation evidence

`preparation/result.json`, `preparation/{initial,final}-transcript/`, fixture before/
after, and hashes prove default model, Python, Podman shell access, exact parent copy,
subsequent agent access, unchanged fixture, full finite history pagination and cleanup.
One preparation agent only; zero scored workers. Preparation had an initial harmless
diagnostic task, then the parent copied before the next harmless probe. The exact
no-task baseline startup is source-supported; not separately exercised because the
one authorized preparation-agent allowance was already used.
