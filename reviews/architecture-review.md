# Architecture Review

Date: 2026-02-22
Scope: Full repository review (not PR diff)

## Executive Summary

- The repository mostly follows the intended layered structure, but there are two architectural contract breaks: application-layer I/O and a non-dyn-compatible `Channel` port that is not used by the gateway.
- Dependency direction is largely clean in production code, but composition/wiring is duplicated across CLI agent, REPL, and gateway, increasing drift risk.
- Testability is strong in many areas (trait-based ports, REPL I/O abstraction), yet gateway runtime code is coupled to concrete implementations instead of domain traits.
- Maintainability risk is moderate due to unbounded session growth and architecture checks that rely on brittle string scanning.

## Findings

| ID | Severity | Title | Evidence | Impact | Recommendation |
|---|---|---|---|---|---|
| F-001 | High | Application layer performs direct filesystem/environment I/O | `src/application/heartbeat.rs:63`, `src/application/heartbeat.rs:68`, `src/application/onboard.rs:30`, `src/application/onboard.rs:33`, `src/application/onboard.rs:37`, `src/application/onboard.rs:63`, `src/application/onboard.rs:76` | Violates AGENTS clean-architecture constraint ("application has no I/O"), makes use-cases harder to test in isolation, and couples business flow to OS/filesystem behavior. | Introduce ports in `domain` (for example `HeartbeatTaskSource`, `OnboardStore`, `PathResolver`) and move file/env/dirs adapters to `infrastructure`; keep `application` purely orchestration. |
| F-002 | High | `Channel` port is not object-safe and is effectively dead architecture | `src/domain/channel.rs:4`, `src/domain/channel.rs:9`, `src/domain/channel.rs:12`, `src/interface/gateway/mod.rs:63`, `src/interface/gateway/services.rs:100` | `Channel` cannot be used as `dyn Channel` (RPITIT methods), contradicts documented trait-object pattern and leads gateway to concrete `TelegramChannel` coupling. This blocks channel substitution and migration-safe extension. | Redefine `Channel` methods to boxed futures (`Pin<Box<dyn Future...>>`) like other domain ports, then thread `Arc<dyn Channel>` through gateway context to decouple interface runtime from Telegram concrete type. |
| F-003 | Medium | Composition root logic is fragmented and duplicated across entrypoints | `src/interface/cli/agent.rs:211`, `src/interface/cli/agent.rs:224`, `src/interface/cli/agent.rs:229`, `src/interface/repl/mod.rs:309`, `src/interface/repl/mod.rs:314`, `src/interface/gateway/mod.rs:217`, `src/interface/gateway/mod.rs:266` | Provider/tool/sandbox wiring is repeated in multiple places, increasing risk of inconsistent behavior and upstream merge conflicts when toolsets or provider policy evolve. | Extract a shared wiring module in `interface` (for example `interface/wiring.rs`) that builds provider stacks and tool registries from mode-specific profiles; keep command handlers thin. |
| F-004 | Medium | Gateway runtime is coupled to concrete implementations instead of ports | `src/interface/gateway/mod.rs:61`, `src/interface/gateway/mod.rs:62`, `src/interface/gateway/mod.rs:63`, `src/interface/gateway/services.rs:27`, `src/interface/gateway/services.rs:28`, `src/interface/gateway/services.rs:100` | Harder to swap implementations (for example in-memory session store, alternate channel), increases mocking friction, and narrows long-term modularity despite having domain traits. | Store trait objects in runtime context (`Arc<dyn AgentLoop>`, `Arc<dyn SessionStore>`, `Arc<dyn Channel>` after fixing F-002) and adapt service signatures to trait-based contracts. |
| F-005 | Low | Session lifecycle is unbounded (memory/disk growth risk) | `src/interface/gateway/services.rs:83`, `src/interface/repl/mod.rs:279`, `src/interface/cli/agent.rs:329`, `src/infrastructure/persistence/session_store.rs:99` | Histories grow without cap; this conflicts with the project’s "ultra-low resource" goal and can degrade latency/storage over long-running use. | Add configurable retention (max messages/tokens per session), periodic compaction/summarization, and enforce truncation before save. |
| F-006 | Low | Architecture boundary test is brittle (string scan, easy blind spots) | `tests/architecture.rs:45`, `tests/architecture.rs:50`, `tests/architecture.rs:59` | Can miss violations via alternate syntax/placement; may give false confidence for boundary enforcement. | Replace/augment with AST-based checks (via `syn`) or compile-time boundary crates/modules; keep current test as a fast smoke check only. |

## Positive Observations

- Layering is explicit and discoverable (`src/domain`, `src/application`, `src/infrastructure`, `src/interface`), and startup flow remains clean through `src/main.rs:1`.
- Most domain ports are well-designed for trait-object usage with boxed futures (`src/domain/provider.rs:26`, `src/domain/tool.rs:31`, `src/domain/session.rs:33`, `src/domain/voice.rs`).
- REPL is highly testable due to abstracted I/O (`BufRead`/`Write`) rather than hardcoded stdio (`src/interface/repl/mod.rs:48`, `src/interface/repl/mod.rs:297`).
- Gateway lifecycle management is disciplined: centralized event-loop context and clean shutdown via `tokio::select!` and task aborts (`src/interface/gateway/mod.rs:78`, `src/interface/gateway/mod.rs:93`, `src/interface/gateway/mod.rs:113`).
- Architecture enforcement exists and is run as part of quality gates (`tests/architecture.rs`).

## Refactoring Roadmap

1. Fix domain port contract first: make `Channel` dyn-compatible and adopt it in gateway runtime context (unblocks decoupling and future channels).
2. Remove I/O from `application`: extract onboarding/heartbeat file access behind domain ports with infra adapters.
3. Consolidate provider/tool/sandbox wiring into one interface-level builder module used by CLI agent, REPL, and gateway.
4. Add session retention policy (configurable cap + truncation before persistence) to align with low-resource goals.
5. Harden architecture tests with AST-based validation to catch import-rule evasions and keep current line-scan test as a fast guard.
