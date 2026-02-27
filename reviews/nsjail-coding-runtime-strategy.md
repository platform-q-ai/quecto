# nsjail Coding Runtime Strategy

## Goal

Add a full-featured coding runtime to Quecto (Pi-like coding capabilities) that can safely run multiple coding jobs in parallel on the same repository, without git state collisions.

Key outcomes:

- Rich coding tool ergonomics (robust edit/replace/diff workflows)
- Strong isolation for security and reliability
- Deterministic parallel execution for multiple issues/tasks
- Minimal impact on existing clean-architecture boundaries

## Decision Summary

1. Use a dedicated **coding worker** runtime, launched per job.
2. Run that worker inside **nsjail** (not just individual shell commands).
3. Use **per-job repo clones** from a local bare mirror (default), not shared git worktrees.
4. Run the coordinator as a **long-lived subagent process** (`quecto agent` child) — not inline in the main agent. The main agent delegates via file-based IPC and the coordinator autonomously manages worker lifecycle.
5. Keep the main agent responsive for user conversation and non-coding tasks while coding jobs run asynchronously.

This gives better fault isolation and security than worktree sharing while preserving good startup performance via mirror clones.

## Why Per-Job Clone Over Worktrees

Worktrees are fast and space-efficient, but share internal git state. In multi-agent parallel execution, shared state increases cross-job interference risk.

Per-job clone advantages:

- Strong isolation: separate `.git` internals per job
- Lower blast radius: one job’s git mistakes do not corrupt others
- Simpler mount model: nsjail gets one writable repo tree only
- Fewer lock/maintenance collisions (refs/index/gc interactions)
- Easy cleanup (`rm -rf` job dir)

Tradeoff:

- More disk and clone overhead than worktrees

Mitigation:

- Clone from a **local bare mirror cache** to keep startup fast

## High-Level Architecture

### Host Coordinator (long-lived subagent process)

The coordinator runs as a **long-lived `quecto agent` child process** — a full subagent with its own LLM, tools, and autonomous agent loop. It is **not** wired inline into the main agent process. The main agent spawns the coordinator on first need and communicates with it via file-based IPC.

Responsibilities:

- Own all worker lifecycle (spawn, poll, kill, restart)
- Allocate job IDs and resource/security policy
- Prepare per-job repo environment
- Launch and monitor nsjail worker processes
- Autonomously reason about job state (stuck workers, failed jobs, retry decisions)
- Proactively notify the main agent of issues (blocked workers, failures, completions)
- Persist artifacts (patches/logs/results) and event logs
- Cleanup job directories based on policy
- Write periodic status snapshots (`coordinator/state.json`) for fast queries

Process model:

- Spawned as a `quecto agent` child process with a coordinator-specific system prompt
- Runs indefinitely (no 120s timeout — configurable long timeout, e.g. 24h)
- Uses a named session for context persistence across restarts
- PID tracked in `coordinator/pid` for liveness checks
- Auto-restarts on next `coding_job` call if it crashes
- Graceful shutdown via signal from main agent

### Control Plane vs Execution Plane

- Main agent = goal planner and user-facing orchestrator (thin delegation to coordinator)
- Host coordinator = autonomous job foreman (long-lived subagent with own LLM loop)
- Sandboxed worker = isolated executor for coding steps only

Rules:

- Worker-to-worker coordination is not allowed directly
- Cross-job decisions are made by the coordinator autonomously, escalating to the main agent only when human input is needed
- Main agent must remain free to converse with users and handle other tool flows while coding jobs execute
- The coordinator does not block the main agent — all communication is asynchronous via file-based IPC

### Sandboxed Coding Worker (inside jail)

Responsibilities:

- Run coding tool loop (read/edit/write/search/shell/git)
- Emit structured tool results/events
- Enforce worker-local safety rules
- Return final result and artifact metadata

Worker IPC:

- JSON Lines over stdin/stdout (worker to coordinator — simple and robust)

## Main Agent ↔ Coordinator IPC (File-Based)

The main agent communicates with the coordinator subagent via file-based IPC. This is chosen over sockets/pipes because it is debuggable (`cat coordinator/inbox/*.json`), survives restarts, requires no protocol, is fully auditable, and aligns with the existing append-only JSONL event log pattern.

### File Layout

```
<workspace>/coordinator/
├── inbox/                    # Main agent writes command files here
│   └── <uuid>.json           # {"action": "run", "repo": "...", ...}
├── outbox/                   # Coordinator writes responses here
│   └── <uuid>.json           # {"ok": true, "job_id": "...", ...}
├── notifications/            # Coordinator writes proactive alerts here
│   └── <ts>_<type>.json      # {"type": "worker_blocked", "job_id": "...", "details": "..."}
├── state.json                # Coordinator status snapshot (alive, job summary, last heartbeat)
└── pid                       # Coordinator process PID for liveness checks
```

### Main Agent Tool Flow (`coding_job`)

The `coding_job` tool in the main agent becomes a **thin file-based delegation layer**:

1. Check if coordinator is alive (read `pid` file, verify process liveness via `kill -0`)
2. If not alive, spawn it as a long-lived subagent (`quecto agent` with coordinator system prompt and full coding tool suite)
3. Write command JSON to `coordinator/inbox/<uuid>.json`
4. Poll `coordinator/outbox/<uuid>.json` for response (with configurable timeout)
5. For fast-path queries (`status`, `list`): read `coordinator/state.json` directly without writing to inbox

### Coordinator Inbox Processing

The coordinator runs a watch/poll loop monitoring `coordinator/inbox/` for new command files. On each new file:

1. Parse the command JSON
2. Execute the requested action (run job, cancel, status query, etc.)
3. Write the response to `coordinator/outbox/<uuid>.json` (same UUID as the command)
4. Remove the processed inbox file

### Coordinator Proactive Notifications

The coordinator writes structured JSON files to `coordinator/notifications/` when issues arise that the main agent should know about without having to ask:

| Type | Trigger |
|---|---|
| `worker_blocked` | Worker asks a question or needs human input |
| `job_failed` | Unexpected crash, timeout, or resource limit hit |
| `worker_stuck` | No progress for N minutes (configurable) |
| `batch_complete` | All jobs in a batch finished (summary ready) |
| `policy_violation` | Worker attempted forbidden action (force push, etc.) |

The main agent checks `coordinator/notifications/` via a tool call or periodic check in the gateway event loop. For urgent notifications, the coordinator can also use `deliver_to` with the Telegram channel to push directly to the user.

Notification format:

```json
{"type": "worker_blocked", "job_id": "...", "question": "Which test framework?", "ts": "..."}
{"type": "job_failed", "job_id": "...", "error": "OOM killed after 2h", "ts": "..."}
{"type": "worker_stuck", "job_id": "...", "no_progress_minutes": 30, "ts": "..."}
{"type": "batch_complete", "job_ids": ["..."], "summary": "3 succeeded, 1 failed", "ts": "..."}
{"type": "policy_violation", "job_id": "...", "detail": "worker attempted force push", "ts": "..."}
```

## Event Storage and State Management (Keep It Simple)

All event storage and state management lives inside the **coordinator subagent process**. The main agent does not directly read or write event logs — it queries job state through file-based IPC (`coordinator/state.json` for fast reads, or inbox/outbox for commands).

Do not introduce Redis, Kafka, or any external queue/store for MVP.

Use a lightweight local-first design (coordinator-internal):

- In-memory queue for live flow: `tokio::mpsc` channels between coordinator and worker supervisors
- Append-only JSONL event log on disk: durable source of truth per run/job (crash recovery + audit)
- Small job index file: current state snapshot (`jobs/index.json`) so status queries are fast
- Artifact directory per job: logs/diffs/summaries referenced by event IDs
- `coordinator/state.json`: periodic snapshot of coordinator liveness and aggregate job summary (read by main agent for fast-path status queries)

Runtime model:

1. Event received in-memory
2. Validate + apply state transition
3. Append event to durable JSONL log (fsync before step 4)
4. Update in-memory state snapshot (periodic compact write to `jobs/index.json`)

Important: `jobs/index.json` is **rebuilt from event logs on startup** and only written as a periodic snapshot for fast status queries. It is not the source of truth — the append-only JSONL logs are.

### Crash Recovery

On coordinator startup:

1. Scan `jobs/` for directories containing `events.jsonl`
2. Replay each event log to reconstruct in-memory state
3. Detect jobs stuck in `running` or `preparing` (coordinator crashed mid-job):
   - Check if nsjail worker process is still alive (by PID from `job.ready` event)
   - If worker is orphaned or dead: transition job to `failed` with `error_code: coordinator_crash`
   - If worker is alive: re-attach event stream and resume monitoring
4. Write recovered state to `jobs/index.json` snapshot
5. Resume normal event processing

Ordering guarantee: event log append is always flushed to disk **before** any in-memory state or index update. This ensures replay produces a consistent state even after unclean shutdown.

### Mirror Locking

Bare mirror updates (`git fetch`) must not collide with concurrent job clones:

- Use a filesystem advisory lock (`flock`) on the mirror directory during `git fetch`
- Clone operations acquire a shared read lock; fetch acquires an exclusive write lock
- If fetch lock is contended, queue the fetch and let in-flight clones complete first
- Stale lock detection via PID check (dead process holding lock = force release)

Rationale:

- Matches Quecto goals (single static binary, low resource usage, minimal operational burden)
- Avoids unnecessary infrastructure complexity for single-node/local deployments
- Keeps observability and crash recovery without external dependencies

Future-only extension (non-MVP):

- Optional distributed coordinator backend (for multi-host or high-throughput deployments)
- Any such backend must preserve the same event schema and replay semantics

## Runtime Lifecycle

Control loop (file-based delegation):

```
main agent --[write inbox]--> coordinator subagent --[spawn nsjail]--> sandboxed workers
main agent <--[read outbox/state/notifications]-- coordinator subagent <--[worker events via stdout]-- sandboxed workers
```

0. **Delegate**
   - Main agent writes command to `coordinator/inbox/<uuid>.json`
   - If coordinator is not alive, main agent spawns it first (auto-spawn)
   - Main agent polls `coordinator/outbox/<uuid>.json` for acknowledgment
1. **Prepare** (coordinator-owned)
   - Ensure mirror exists/updated
   - Create job directory
   - Clone repo into `jobs/<job-id>/repo`
   - Checkout requested base branch/commit
   - Create job branch (e.g., `quecto/job/<job-id>`)
2. **Execute** (coordinator-owned)
   - Start worker in nsjail with strict mounts and limits
   - Process coding plan/tool calls until completion or timeout
   - Coordinator autonomously monitors progress and handles stuck/failed workers
3. **Export** (coordinator-owned)
   - Capture patch (`git diff`), commit metadata, logs, test output
   - Return summary + artifact references + goal-progress signals for next decision
4. **Notify**
   - Coordinator writes completion/failure notification to `coordinator/notifications/`
   - For urgent issues, coordinator can push directly to user via `deliver_to` + Telegram channel
5. **Finalize** (coordinator-owned)
   - Optionally keep branch/artifacts
   - Cleanup job directory (default on success)

## Security Model

### nsjail Defaults

- `no_new_privs` enabled
- seccomp-bpf profile applied
- cgroups limits enforced
- wall timeout + kill
- PID limit
- read-only root where possible
- writable bind mount only for job directory

### Network Policy

Default: deny all egress.

Optional allowlist per job/profile for known package and git hosts.

### Secrets Handling

- Inject minimal credentials only when required
- Scope credentials to process/job lifetime
- Redact sensitive strings in logs/tool output
- Never store raw secrets in artifacts

## Coding Capability Parity Target (Pi-Like)

Implement these first-class tools in worker runtime:

- `read_file`
  - line offset/limit pagination
  - truncation metadata + continuation hints
- `write_file`
  - atomic write and parent mkdir
  - size limits
- `edit_file`
  - exact replace mode
  - fuzzy fallback mode
  - CRLF/LF normalization
  - BOM-safe behavior
  - smart punctuation normalization fallback
  - ambiguity detection (multiple potential matches)
  - no-op detection
  - return unified diff + first changed line
- `edit_preview`
  - compute diff without writing
- `grep_content`
  - fast regex search, `.gitignore` aware
- `find_files`
  - glob search, `.gitignore` aware
- `list_dir`
  - bounded listing with stable ordering
- `exec`
  - timeout, streamed output draining, truncation + spill artifacts
- safe git wrappers
  - status/diff/add/commit/branch operations
  - block destructive commands by default

## GitHub Management Boundary

GitHub and PR operations should be owned by the host coordinator, not by sandbox workers.

### Main Agent

- Decides when to publish work externally (open/update PR, request review, merge)
- Sets intent and constraints (target branch, reviewers, labels, merge policy)

### Host Coordinator

- Executes GitHub actions from approved intent:
  - push job branches when allowed
  - create/update pull requests
  - link issues, apply labels, request reviewers
  - fetch PR status/checks and report back
- Enforces safety and policy gates:
  - default no force-push
  - protected branch awareness
  - credential scope and redaction
  - repository allowlist and branch naming policy
- Aggregates multi-job output into a single decision-ready publish/update recommendation

### Sandboxed Worker

- Performs local repo work only (code/test/local commits)
- Produces artifacts for coordinator handoff (`patch.diff`, `summary.json`, logs)
- Does not call GitHub APIs directly and does not manage PR lifecycle

## Child Agent Orchestration

Sub-workers may request child agents for specialized tasks (for example: security review, performance review, architecture review, docs updates), but spawning is coordinator-controlled.

### Worker Behavior

- Worker can emit a `spawn_request` event with purpose, scope, and expected output
- Worker cannot launch arbitrary child processes directly
- Worker receives child-agent results as structured artifacts/events from coordinator

### Coordinator Policy

- Enforce an allowlist of child agent types
- Enforce max spawn depth (default: 1)
- Enforce per-job spawn limits and resource/time budgets
- Deduplicate equivalent spawn requests where possible
- Route child-agent output back into parent job context and aggregate for main-agent decisions

### Safety Rules

- Child agents follow the same isolation and policy model as normal workers
- External side effects (push, PR create/update, merge actions) remain coordinator-only
- Child-agent runs are fully audited in artifacts (`spawn_log.json`)

## Skill Injection Policy

Coordinator has skill access and is responsible for controlled skill injection into worker jobs.

### Source of Truth

- Coordinator resolves skills from trusted workspace sources and policy allowlists
- Coordinator snapshots applied skill content at job start for reproducibility

### Injection Model

- Effective skill set = global defaults + profile skills + task-specific skills
- Skills are injected into worker system context at launch
- Workers can suggest additional skills, but coordinator must approve before applying

### Guardrails

- Optional per-profile skill denylist/allowlist
- No arbitrary runtime loading of unknown remote skills from inside workers
- All applied skills are recorded in artifacts (`skills_applied.json`)

## Prompt and Context Strategy

Keep Quecto prompt philosophy minimal, add coding profile guidance:

- read before edit
- prefer targeted edits over rewrites
- use edit preview for risky or ambiguous changes
- run verification commands before finalizing
- workers may propose local strategy updates, but final cross-job direction comes from main agent

Context management integration:

- preserve recent diffs, failed edit diagnostics, test failures
- spill large tool outputs via existing context spill store + recall IDs
- maintain concise per-job status snapshots so main agent can continue normal user conversation without blocking

## Long-Lived Subagent Requirements

The current `SpawnTool` has a 120-second timeout and discards stdout/stderr. The coordinator subagent needs different semantics:

1. **No timeout (or configurable long timeout, e.g. 24h)** — the coordinator runs indefinitely while jobs are active
2. **Graceful shutdown** — main agent can signal the coordinator to shut down cleanly (e.g. via a `shutdown` command in inbox, or SIGTERM)
3. **Auto-restart** — if coordinator dies, main agent re-spawns it on next `coding_job` call
4. **Session persistence** — coordinator uses a named session so it survives restarts with context
5. **PID tracking** — main agent writes/reads `coordinator/pid` for liveness checks (`kill -0`)
6. **No stdout/stderr capture by main agent** — coordinator manages its own I/O; communication is exclusively via file-based IPC

### Reuse of Existing Infrastructure

| Component | Reuse Strategy |
|---|---|
| `SpawnTool` | Extend with long-lived mode (no 120s timeout) |
| `SubagentConfig` | Add coordinator-specific fields or use `system` prompt override |
| `CodingCoordinator` | Moves unchanged into coordinator process |
| `CodingLifecycleDriver` | Moves unchanged into coordinator process |
| `NsjailWorkerRuntime` | Moves unchanged into coordinator process |
| `CodingJobTool` | Rewritten as thin delegation layer for main agent; original logic moves to coordinator |
| `CoordinatorBus` | Replaced by file-based IPC (inbox/outbox/notifications) or kept for coordinator-internal use |
| `Channel` trait + `deliver_to` | Coordinator uses for urgent Telegram notifications |
| Event logs (JSONL) | Unchanged, owned by coordinator |
| Crash recovery | Runs in coordinator process on startup |

## Clean Architecture Placement

### domain/

- `coding_job.rs`
  - `CodingJobSpec`, `CodingJobLimits`, `CodingJobPolicy`, `CodingJobResult`
- `coding_worker.rs`
  - worker request/response/event model traits
- `coding_coordinator.rs`
  - coordinator command/response/notification types, `CoordinatorCommand`, `CoordinatorNotification`

### application/

- `coding_orchestrator.rs`
  - job lifecycle state machine (runs inside coordinator subagent process)
- `coding_policy.rs`
  - policy resolution (security/network/toolset)
- `coding_todos.rs`
  - coordinator-owned per-worker todo state and aggregation
- `coding_crash_recovery.rs`
  - event log replay and orphaned job detection on coordinator startup

### infrastructure/

Components that run **inside the coordinator subagent process**:

- `sandbox/nsjail_worker.rs`
  - launch/monitor/stop worker process
- `vcs/job_repo.rs`
  - mirror management, per-job clone creation, branch setup, mirror locking
- `vcs/github_ops.rs`
  - PR lifecycle and repository operations under policy control
  - named `github_ops` (not `github`) to avoid confusion with `interface/gateway/`
- `coding/child_orchestrator.rs`
  - validated child-agent spawning, budget enforcement, result routing
  - reuses existing `application/subagent.rs` (`SubagentContext`) for spawn mechanics
  - adds policy layer (allowlist, depth/budget caps, dedup) on top of existing spawn infra
- `coding/coordinator_bus.rs`
  - inbox watcher, outbox writer, notification writer, state.json snapshot writer
- `tools/coding/*`
  - worker coding tool implementations (wired into coordinator's tool registry)
- `persistence/coding_artifacts.rs`
  - artifact manifest and metadata writing
- `persistence/coding_events.rs`
  - append-only JSONL event log, index snapshot, replay-on-startup

Components that run **inside the main agent process**:

- `tools/coding_delegation.rs`
  - `CoordinatorDelegationTool` — thin file-based delegation layer
  - writes commands to `coordinator/inbox/`, polls `coordinator/outbox/`
  - reads `coordinator/state.json` for fast-path status queries
  - checks `coordinator/notifications/` for proactive alerts
  - auto-spawns coordinator subagent if not alive (PID check + `kill -0`)
- `coding/coordinator_spawn.rs`
  - coordinator process spawning logic (long-lived `quecto agent` with coordinator system prompt)
  - PID file management and liveness verification
  - graceful shutdown signaling

### interface/

- CLI wiring for debugging/ops:
  - `quecto coding run ...`
  - `quecto coding status ...`
  - `quecto coding cleanup ...`

## Configuration Proposal

Add `coding` section to config:

- `coding.enabled`
- `coding.isolation.profile` (`strict` default; nsjail is the only supported mode — do not add a mode selector)
- `coding.isolation.network.default` (`deny` default)
- `coding.isolation.network.allow_hosts[]`
- `coding.isolation.resources` (`max_memory_mb`, `max_cpu_seconds`, `max_wall_seconds`, `max_pids`)
- `coding.repos.cache_dir` (bare mirrors; `jobs_dir` defaults to `<cache_dir>/jobs/` — only expose if separate volume needed)
- `coding.repos.clone_strategy` (`mirror_clone` default)
- `coding.repos.cleanup_policy` (enum: `always`, `on_success`, `on_failure`, `never`; default: `on_success`)
- `coding.coordinator.system_prompt` (override coordinator system prompt; defaults to built-in coordinator prompt)
- `coding.coordinator.model` (LLM model for coordinator; defaults to global model)
- `coding.coordinator.max_timeout_seconds` (coordinator process lifetime; default: 86400 = 24h)
- `coding.coordinator.inbox_poll_interval_ms` (how often coordinator checks inbox; default: 1000)
- `coding.coordinator.state_snapshot_interval_seconds` (how often state.json is written; default: 10)
- `coding.coordinator.stuck_worker_threshold_minutes` (no-progress threshold for `worker_stuck` notification; default: 30)
- `coding.orchestration.max_parallel_jobs`
- `coding.orchestration.todos.enabled`
- `coding.orchestration.todos.max_items_per_job`
- `coding.orchestration.child_agents.allow_types[]`
- `coding.orchestration.child_agents.max_depth`
- `coding.orchestration.child_agents.max_spawns_per_job`
- `coding.orchestration.child_agents.default_timeout_seconds`
- `coding.skills.enable_injection`
- `coding.skills.default[]`
- `coding.skills.allowlist[]`
- `coding.skills.denylist[]`
- `coding.tools.edit.fuzzy_match` (worker tool config — overrides inherited defaults when set)
- `coding.tools.exec.timeout_seconds` (worker tool config — overrides inherited defaults when set)
- `coding.artifacts.store_dir`

Config simplification notes:

- `coding.isolation.mode` is intentionally omitted — nsjail is the only supported isolation mode. Do not add a selector.
- `coding.repos.jobs_dir` is derived from `cache_dir` by default. Only add to config if deploying jobs to a separate volume.
- `coding.repos.cleanup_on_success` / `cleanup_on_failure` are replaced by a single `cleanup_policy` enum.
- Worker tool config keys (`coding.tools.*`) act as overrides. When absent, the worker inherits from the existing global `tools.*` config.

## Phased Delivery Plan

### Phase 1: Foundation

- Worker process + JSONL RPC
- nsjail launcher and lifecycle hooks
- mirror cache + per-job clone setup (including mirror flock protocol)
- baseline tools: read/write/list/find/grep/exec
- single-job end-to-end path
- **Coordinator as long-lived subagent**: spawn coordinator as a `quecto agent` child process with file-based IPC (inbox/outbox/notifications/state.json/pid). Main agent delegates via thin `CoordinatorDelegationTool`. This is a prerequisite for all later phases — do not defer.
- **SpawnTool long-lived mode**: extend `SpawnTool` to support long-lived subagents (no 120s timeout, PID tracking, auto-restart on next call)
- **Coordinator auto-spawn and liveness**: main agent checks `coordinator/pid`, spawns if not alive, writes PID file

### Phase 2: Edit Engine Parity

- robust `edit_file` with diagnostics and normalization
- `edit_preview`
- unified diff generation and first-changed-line metadata

### Phase 3: Git-Aware Coding Flow

- safe git wrapper tools
- branch-per-job default conventions
- artifact export bundle (`patch.diff`, `run.log`, `summary.json`)

### Phase 4: Parallel Orchestration

- multi-job queue and resource scheduling
- per-job policy overrides
- host-level concurrency caps
- coordinator autonomous decision-making for stuck/failed workers (retry, reassign, escalate)
- coordinator-only merge point for cross-job conflict detection and resolution
- coordinator-owned per-worker todo lists + global dependency board
- proactive notification pipeline (worker_blocked, job_failed, worker_stuck, batch_complete, policy_violation)

### Phase 5: GitHub Orchestration

- coordinator-managed branch publish and PR creation/update
- issue/PR linkage and reviewer/label automation hooks
- status/check ingestion for main-agent decision loop
- policy enforcement tests (protected branches, no force-push, auth scope)

### Phase 6: Child Agents and Skills

- validated child-agent spawn pipeline (allowlist, depth/budget caps)
- coordinator skill resolution and injection snapshots
- artifact audit logs for spawns and applied skills
- tests for policy enforcement and non-blocking coordinator behavior

### Phase 7: Security Hardening

- stricter seccomp profiles
- network allowlist profiles
- stronger secret redaction and leakage tests
- denylist tuning and abuse resistance

### Phase 8: Observability and DX

- structured tracing for tool calls and resource usage
- job replay/debug utilities
- CLI inspection and cleanup commands

## Testing Strategy

### Unit Tests

- edit matching edge cases (CRLF/BOM/smart punctuation)
- ambiguity/no-op detection
- diff correctness
- repo manager branch and clone setup

### Integration Tests

- nsjail process lifecycle and timeout kill
- mount boundary enforcement
- parallel jobs on same upstream repo without collisions

### Security Tests

- path traversal and symlink escape prevention
- blocked command patterns and injection attempts
- environment secret non-leakage

### BDD Tests

- multi-file refactor scenario
- failed edit recovery with alternative strategy
- parallel issue implementation (N jobs)
- artifact generation and retrieval
- crash recovery: coordinator restart replays event logs and recovers job state
- cancel/timeout: job cancellation emits correct events and reaches terminal state
- child agent: worker requests security review, coordinator approves, child returns findings
- skill injection: coordinator snapshots skills at job start, worker receives correct context
- GitHub publish: coordinator creates PR from job artifacts under policy constraints
- todo lifecycle: coordinator tracks per-worker todos through full status cycle
- main agent responsiveness: agent handles user messages while coding job runs in background
- coordinator auto-spawn: main agent spawns coordinator on first `coding_job` call if not alive
- coordinator auto-restart: main agent detects dead coordinator and re-spawns on next call
- file-based IPC round-trip: command written to inbox, response read from outbox
- proactive notifications: coordinator writes notification for blocked/failed worker, main agent reads it
- coordinator graceful shutdown: main agent signals shutdown, coordinator finishes active jobs and exits
- coordinator state.json: fast-path status query reads state.json without inbox/outbox round-trip

## Risks and Mitigations

- **Disk growth from job clones**
  - Mitigation: mirror clone + cleanup policy + retention TTL
- **Network needs for builds conflict with default deny**
  - Mitigation: profile-based allowlist, explicit opt-in
- **Complexity in worker protocol evolution**
  - Mitigation: versioned JSON schema and compatibility checks
- **Performance overhead vs direct host execution**
  - Mitigation: parallelism, mirror cache, per-profile resource tuning
- **Mirror lock contention under parallel clones**
  - Mitigation: flock-based read/write locking (see Mirror Locking section); queue fetch behind active clones
- **Index file (`jobs/index.json`) write contention**
  - Mitigation: index is rebuilt from event logs on startup; only written as periodic snapshot, not on every event
- **Main agent responsiveness requires async integration**
  - Mitigation: coordinator runs as a long-lived subagent process with file-based IPC — main agent is never blocked. Implemented in Phase 1 as a prerequisite.
- **Coordinator subagent process crashes or hangs**
  - Mitigation: main agent checks PID liveness on every `coding_job` call; auto-restarts coordinator if dead. Coordinator uses named session for context persistence across restarts. Crash recovery replays event logs to reconstruct state.
- **File-based IPC latency and stale files**
  - Mitigation: polling with configurable timeout; cleanup of processed inbox files; `state.json` includes last-heartbeat timestamp for staleness detection. File-based IPC trades latency for debuggability and restart-safety.
- **Coordinator LLM cost from autonomous loop**
  - Mitigation: coordinator only invokes LLM when it needs to reason about exceptions (stuck workers, retry decisions). Routine lifecycle transitions (prepare, execute, export) are mechanical state machine steps that do not require LLM calls.
- **Child agent spawning overlaps existing `SubagentContext`**
  - Mitigation: reuse existing spawn infrastructure; add policy layer on top, do not duplicate spawn mechanics
- **JSONL event size unbounded**
  - Mitigation: 1 MiB hard cap per event line (defined in contract); truncate/spill oversized tool outputs

## Operational Defaults (Recommended)

- `clone_strategy = mirror_clone`
- `network.default = deny`
- `cleanup_policy = on_success`
- `max_parallel_jobs = 5`
- strict git safety (no force push, no destructive reset)

## Future Extensions

- Optional fast mode using worktrees (explicitly non-default)
- WASM-hosted coding tools for non-shell operations
- Remote execution backends beyond nsjail for stronger tenant isolation

## Briefing Notes for LLM Engineer

When implementing, optimize for:

1. Reliability and isolation over raw speed (default mode)
2. High-quality edit diagnostics (this is core to coding UX)
3. Deterministic behavior and clear artifacts for every run
4. Strict adherence to existing clean architecture boundaries
5. Incremental delivery with test coverage at each phase

Definition of done for MVP:

- Coordinator runs as a long-lived subagent process (`quecto agent` child), not inline in main agent
- Main agent delegates to coordinator via file-based IPC (inbox/outbox/notifications/state.json/pid)
- Coordinator auto-spawns on first `coding_job` call and auto-restarts if it crashes
- Parallel jobs can safely implement multiple issues in same repo using per-job clones
- Worker runs inside nsjail with bounded resources and mount restrictions
- Edit tool supports robust replace semantics and returns useful diffs
- Artifacts/logs are persisted and inspectable
- Existing Quecto flows remain backward compatible
- Main agent stays responsive for user conversation and non-coding requests while coding jobs run
- Coordinator proactively notifies main agent of blocked/failed/stuck workers
- Coordinator returns decision-ready aggregate job status back to main agent (not only raw artifacts)
