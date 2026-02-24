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
4. Keep orchestration in Quecto host process; keep coding execution in sandbox worker.
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

### Host Coordinator (outside jail)

Responsibilities:

- Receive coding task requests
- Allocate job IDs and resource/security policy
- Prepare per-job repo environment
- Launch and monitor nsjail worker process
- Stream worker events/results to agent loop
- Persist artifacts (patches/logs/results)
- Cleanup job directories based on policy

### Control Plane vs Execution Plane

- Main agent = goal planner and user-facing orchestrator
- Host coordinator = job foreman (queue, lifecycle, policy, aggregation)
- Sandboxed worker = isolated executor for coding steps only

Rules:

- Worker-to-worker coordination is not allowed directly
- Cross-job decisions are made by main agent, using coordinator summaries
- Main agent must remain free to converse with users and handle other tool flows while coding jobs execute

### Sandboxed Coding Worker (inside jail)

Responsibilities:

- Run coding tool loop (read/edit/write/search/shell/git)
- Emit structured tool results/events
- Enforce worker-local safety rules
- Return final result and artifact metadata

IPC:

- JSON Lines over stdin/stdout (simple and robust)

## Event Storage and State Management (Keep It Simple)

Do not introduce Redis, Kafka, or any external queue/store for MVP.

Use a lightweight local-first design:

- In-memory queue for live flow: `tokio::mpsc` channels between coordinator and worker supervisors
- Append-only JSONL event log on disk: durable source of truth per run/job (crash recovery + audit)
- Small job index file: current state snapshot (`jobs/index.json`) so status queries are fast
- Artifact directory per job: logs/diffs/summaries referenced by event IDs

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

Control loop:

`main agent -> host coordinator -> sandboxed workers -> host coordinator -> main agent`

1. **Prepare**
   - Ensure mirror exists/updated
   - Create job directory
   - Clone repo into `jobs/<job-id>/repo`
   - Checkout requested base branch/commit
   - Create job branch (e.g., `quecto/job/<job-id>`)
2. **Execute**
   - Start worker in nsjail with strict mounts and limits
   - Process coding plan/tool calls until completion or timeout
3. **Export**
   - Capture patch (`git diff`), commit metadata, logs, test output
   - Return summary + artifact references + goal-progress signals for next decision
4. **Finalize**
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

## Clean Architecture Placement

### domain/

- `coding_job.rs`
  - `CodingJobSpec`, `CodingJobLimits`, `CodingJobPolicy`, `CodingJobResult`
- `coding_worker.rs`
  - worker request/response/event model traits

### application/

- `coding_orchestrator.rs`
  - job lifecycle state machine
- `coding_policy.rs`
  - policy resolution (security/network/toolset)
- `coding_todos.rs`
  - coordinator-owned per-worker todo state and aggregation

### infrastructure/

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
- `tools/coding/*`
  - worker coding tool implementations
- `persistence/coding_artifacts.rs`
  - artifact manifest and metadata writing
- `persistence/coding_events.rs`
  - append-only JSONL event log, index snapshot, replay-on-startup

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
- **Prototype non-blocking agent integration**: coordinator runs as async background task via `tokio::spawn`, communicates with main agent through channels, so main agent loop remains responsive. This is a prerequisite for all later phases — do not defer.

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
- non-blocking coordination so main agent can interleave conversation and unrelated tasks
- coordinator-only merge point for cross-job conflict detection and resolution
- coordinator-owned per-worker todo lists + global dependency board

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
  - Mitigation: prototype non-blocking coordinator in Phase 1 (not Phase 4); coordinator as `tokio::spawn` task with channel IPC to agent loop
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

- Parallel jobs can safely implement multiple issues in same repo using per-job clones
- Worker runs inside nsjail with bounded resources and mount restrictions
- Edit tool supports robust replace semantics and returns useful diffs
- Artifacts/logs are persisted and inspectable
- Existing Quecto flows remain backward compatible
- Main agent stays responsive for user conversation and non-coding requests while coding jobs run
- Coordinator returns decision-ready aggregate job status back to main agent (not only raw artifacts)
