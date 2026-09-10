# Issue #1836 discovery evidence

Inspected live GitHub and repository at `master`/`bbcd01d32da78d22b29771bc9142ef22fd38583b` (local checkout matched remote and was clean before evidence creation).

## Live issue state

- [#1835](https://github.com/platform-q-ai/quecto/issues/1835), **List environments**: OPEN epic; no assignee, no comments; one native child (#1836), 0% complete. It establishes `application/environments/` as the authoritative owner and requires a clean cut without compatibility paths.
- [#1836](https://github.com/platform-q-ai/quecto/issues/1836), **Migrate list-environments query to application/environments**: OPEN; no assignee, labels, or comments; native parent #1835. No prerequisite is declared.
- Timeline evidence contains only parent/child and cross-reference events (plus the epic label); there is no claim event/comment.

## Exact bounded scope

Move `EnvironmentControlUseCase::get_containers` from `src/application/environment_control.rs` to a synchronous application query at `src/application/environments/list_environments.rs`; wire the public allowlisted `agent_cmd {"agent_id":"*","command":"get_containers"}` route to it using the same session `EnvironmentRegistry` as spawn/monitor/kill. Preserve complete inventory/JSON shape, registry iteration order, lifecycle visibility, detached snapshots, poison recovery, and current path rendering. Migrate callers/tests/architecture assertions and remove the old listing method plus `src/lib.rs::environment_control_app` alias after canonicalizing remaining kill imports. Retain kill behavior, registry lifecycle ownership, adapter presentation, and `EnvironmentRegistry::entries()`. No global runtime discovery, new endpoint, async machinery, sorting redesign, compatibility shim, forwarding facade, or duplicate authority.

## Repository instructions

The sole `AGENTS.md` requires rigorous Clean Architecture, TDD/BDD red-green-refactor, defensive coverage of all paths/edges, assertions on critical invariants, and affirmative allowlist guards only.

## Open PR / conflict and claim state

Live open-PR enumeration returned only [#1400](https://github.com/platform-q-ai/quecto/pull/1400), **Add rust AST graph tool**. It neither references/closes #1835/#1836 nor claims their work. No open implementation PR or GitHub-visible claimant exists for #1836.

PR #1400 overlaps one #1836 hotspot, `quecto-agentic-harness/src/infrastructure/extensions/native.rs`, but its patch is in official-tool registration (adding `rust_ast_graph`); #1836 changes later environment registry/query wiring. Thus current direct conflict risk is low but a same-file rebase may be required. #1836 itself is intentionally atomic/high-coupling across `application/mod.rs`, `lib.rs`, `agent_cmd.rs`, `agent_cmd_containers.rs`, `native.rs`, `tests/architecture.rs`, and shared registry contracts, so parallel partial implementation is unsafe.

## Automation discovered

Repository workflows present: `.github/workflows/ci.yml`, `binary-release.yml`, and `reset-merge-requested.yml`; detailed automation safety analysis belongs to board task 3.
