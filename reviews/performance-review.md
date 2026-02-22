# Performance Review

Date: 2026-02-22
Scope: Full repository review (not PR diff)

## Executive Summary

- Several hot-path operations still do blocking disk/process work inside async tasks, which can stall Tokio worker threads under load.
- The largest regressions are in command execution (`exec`) and cron persistence: potential pipe deadlock/timeouts and repeated full-file rewrite patterns can amplify latency quickly.
- Session/message history is unbounded across gateway/REPL flows, so both memory footprint and per-request LLM latency grow with uptime.
- Streaming/provider paths currently buffer whole responses and clone large request payloads, leaving throughput and backpressure headroom on the table.

## Findings

| ID | Severity | Title | Evidence | Performance impact | Recommendation |
|---|---|---|---|---|---|
| PERF-001 | High | `exec` waits before draining stdout/stderr (pipe backpressure risk) | `src/infrastructure/tools/exec.rs:139`, `src/infrastructure/tools/exec.rs:142`, `src/infrastructure/tools/exec.rs:143`, `src/infrastructure/tools/exec.rs:180`, `src/infrastructure/tools/exec.rs:193` | Child processes that write enough output can block on full pipes before exit; parent waits on `child.wait()`, causing false timeouts, retries, and wasted process churn on a hot tool path. | Read stdout/stderr concurrently while process runs (`tokio::io::copy`/tasks or `wait_with_output` + bounded output strategy), and enforce output caps to avoid memory spikes. |
| PERF-002 | High | Cron persistence is full-file, sync I/O, rewritten multiple times per tick | `src/infrastructure/persistence/cron_store.rs:47`, `src/infrastructure/persistence/cron_store.rs:51`, `src/infrastructure/persistence/cron_store.rs:58`, `src/infrastructure/persistence/cron_store.rs:68`, `src/application/cron_executor.rs:29`, `src/application/cron_executor.rs:41`, `src/interface/gateway/services.rs:203`, `src/interface/gateway/services.rs:212` | Every due job triggers multiple `load_all`/`save_all` passes over the entire JSON file using `std::fs`, inside a 2s loop; cost scales with job count and can block runtime threads and increase tail latency. | Move cron store to async I/O with batched updates per tick (single read + single write), or keep an in-memory index with periodic durable flush; avoid per-job full-file rewrites. |
| PERF-003 | High | Conversation history grows unbounded and is fully replayed/saved | `src/application/agent_loop.rs:90`, `src/application/agent_loop.rs:118`, `src/application/agent_loop.rs:148`, `src/interface/gateway/services.rs:34`, `src/interface/gateway/services.rs:83`, `src/infrastructure/persistence/session_store.rs:99` | Memory, serialization cost, and token usage increase linearly with session age; long-lived chats will see rising latency and higher provider cost. | Add history windowing/summarization (token-based cap), and persist compact summaries + recent turns; optionally cap tool transcript payloads. |
| PERF-004 | Medium | Streaming providers buffer full SSE responses in memory (no incremental backpressure) | `src/infrastructure/providers/openai.rs:170`, `src/infrastructure/providers/openai.rs:175`, `src/infrastructure/providers/anthropic.rs:198`, `src/infrastructure/providers/anthropic.rs:203` | Large streams allocate a full response string before parse, increasing peak RSS and latency to first usable output; this weakens streaming gains under heavy responses. | Parse SSE incrementally from byte stream/chunks; emit/assemble deltas progressively and apply bounded buffers for tool argument accumulation. |
| PERF-005 | Medium | Blocking filesystem calls inside async tool execution paths | `src/infrastructure/tools/filesystem.rs:69`, `src/infrastructure/tools/filesystem.rs:125`, `src/infrastructure/tools/filesystem.rs:129`, `src/infrastructure/tools/filesystem.rs:187`, `src/infrastructure/tools/filesystem.rs:198`, `src/infrastructure/tools/filesystem.rs:312` | File tools run via async trait interfaces but perform `std::fs` operations, potentially blocking Tokio workers during larger file operations and reducing concurrency. | Switch to `tokio::fs` (or `spawn_blocking` for unavoidable sync APIs) and add file-size guards/streamed reads where practical. |
| PERF-006 | Medium | Voice update handling can spawn unbounded tasks | `src/interface/gateway/telegram.rs:134`, `src/interface/gateway/telegram.rs:140`, `src/interface/gateway/telegram.rs:151` | Burst voice traffic creates one detached task per update; can grow memory/task scheduling overhead and contend for outbound/inbound channels. | Gate with a semaphore or bounded worker pool for transcription/download tasks; return overload responses when saturated. |
| PERF-007 | Medium | Inbound agent processing is single-threaded, causing head-of-line blocking | `src/interface/gateway/services.rs:31`, `src/interface/gateway/services.rs:42`, `src/interface/gateway/services.rs:48` | One slow request blocks later messages in the same global queue, reducing throughput and increasing latency under concurrent chats. | Partition by chat/session (per-key workers) or use bounded parallelism with ordering guarantees per session key. |
| PERF-008 | Low | Repeated cloning of large request/session vectors on hot provider/session paths | `src/infrastructure/providers/fallback.rs:159`, `src/infrastructure/providers/fallback.rs:160`, `src/infrastructure/providers/openai.rs:302`, `src/infrastructure/providers/anthropic.rs:340`, `src/interface/gateway/services.rs:83` | Extra allocations/copies scale with message/tool list size; adds CPU/RAM overhead on every request even when not needed. | Refactor trait boundaries to reduce ownership cloning (borrow where possible, use `Arc<[T]>`/shared immutable request snapshots). |

## Positive Observations

- Bounded channel capacity is in place (`MessageBus::new(256)`), preventing unbounded queue growth in core gateway message passing (`src/interface/gateway/mod.rs:277`).
- Health server includes concurrency limiting and read timeout, reducing slowloris/resource exhaustion risk (`src/infrastructure/health/server.rs:70`, `src/infrastructure/health/server.rs:98`, `src/infrastructure/health/server.rs:101`).
- Voice payloads enforce maximum size checks before full download, limiting worst-case memory use (`src/interface/gateway/telegram.rs:12`, `src/interface/gateway/telegram.rs:171`, `src/infrastructure/channels/telegram.rs:260`).
- Tool loop has hard iteration caps, containing runaway tool-call loops (`src/application/agent_loop.rs:14`, `src/application/agent_loop.rs:129`).
- Subagent/exec flows include timeout and kill behavior to avoid orphan processes (`src/infrastructure/tools/spawn.rs:132`, `src/infrastructure/tools/exec.rs:162`).

## Benchmark and Profiling Plan

1. Add `tokio-console` plus tracing spans around `run_inbound_processor`, `cron_tick`, and file tools to quantify blocked worker time and queue wait.
2. Build microbenchmarks for `exec` with large stdout/stderr (for example 1MB and 10MB) to validate deadlock/timeout behavior and compare concurrent-drain fixes.
3. Stress cron with N={10,100,1000} jobs and measure tick duration, file I/O bytes, and p95 latency before and after batched persistence.
4. Run long-session replay tests (for example 1k and 5k turns) to track RSS, serialization time, and provider call latency growth from unbounded history.
5. Profile provider streaming on large SSE outputs (heap profile plus CPU flamegraph) to validate gains from incremental parsing and reduced cloning.

## Post-Merge Delta Addendum

Date: 2026-02-22
Scope: Delta review from `aafeeda..256fc2b` (provider API-base hardening and wiring updates)

### Executive Summary

- No material runtime performance regression found in the merged security/provider changes.
- New provider validation is on provider-construction paths (startup/CLI build), not message hot paths, so steady-state agent throughput is unchanged.
- Fail-fast provider configuration errors can reduce wasted retries/startup churn when configuration is invalid.

### Delta Findings

| ID | Severity | Title | Evidence | Performance impact | Recommendation |
|---|---|---|---|---|---|
| PERF-D1 | Low | Repeated env var lookup during provider base validation | `src/infrastructure/providers/mod.rs:43`, `src/infrastructure/providers/mod.rs:50`, `src/infrastructure/providers/mod.rs:96`, `src/infrastructure/providers/mod.rs:142`, `src/interface/cli/agent.rs:396`, `src/interface/gateway/mod.rs:266` | `std::env::var(QUECTO_ALLOW_CUSTOM_PROVIDER_HOSTS)` is evaluated in host validation; cost is small and currently on provider construction only, but it is repeated when building multiple providers/entrypoints. | Optional micro-optimization: cache the opt-in flag once (e.g., `OnceLock<bool>`) inside the providers module if startup latency becomes measurable. |

### Delta Strengths

- Provider validation now fails before adapter construction/network I/O, reducing wasted work on invalid configurations (`src/infrastructure/providers/mod.rs:142`).
- Composition roots now surface explicit config errors instead of silently dropping providers, improving operator feedback and reducing misconfiguration retry loops (`src/interface/cli/agent.rs:399`, `src/interface/gateway/mod.rs:269`).
- Host allowlist checks are constant-time string comparisons and do not introduce new async blocking points (`src/infrastructure/providers/mod.rs:54`, `src/infrastructure/providers/mod.rs:95`).

### Targeted Follow-Up Benchmark

1. Add a small startup benchmark for provider creation paths (`create_provider`, CLI provider build, gateway fallback build) with valid vs invalid `api_base` to confirm negligible overhead under representative configurations.
