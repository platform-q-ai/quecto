# ADR-0021: Script-Managed Subagent Launch Uses Clean Architecture Ports

**Status:** Accepted

## Context

Issue #1369 Slice 1 adds script-managed subagent creation while preserving the existing local spawn protocol and ADR-0020's single bundled-native tool model.

## Decision

- Domain owns pure launch vocabulary only: launch intent/result, typed environment/ref/endpoint identities, and ports. It must not expose process, filesystem, environment, executable parsing, script execution, or adapter JSON details.
- Application owns the launch transaction: prepare, create, readiness, register, initial prompt, commit, and rollback.
- Infrastructure owns concrete adapters. Local process launch is one adapter; configured script-managed create is another. Script argv construction, process execution, JSON/contract parsing, repository discovery, path validation, and direct socket mechanics stay there.
- Interface remains thin. `spawn` parses tool input and delegates to the application use case; UDS handlers decode/delegate/encode and do not own environment transactions or cleanup orchestration.
- Composition wires one session-scoped environment registry into launch/environment services and protocol surfaces.
- The tool model remains ADR-0020: `spawn` and `agent_cmd` are normal bundled-native tools with existing policy/catalogue/execution semantics.
- Core Rust is runtime-agnostic. It invokes configured executables and consumes the documented JSON contract; Docker/Podman/devcontainer details belong in external scripts.
- Vocabulary maps to `quecto-runtime-manager` where it coincides: repository checkout and runtime envelope describe script-owned workspace/environment data. Session-local `CN`/environment refs are Quecto registry identities, distinct from script/runtime IDs.

## Consequences

Architecture tests enforce that domain/application do not perform runtime I/O or process construction. Public domain/application launch ports require shared behavioral contract tests under `tests/contracts/`. Future slices extend the same registry and ports rather than adding parallel launch pipelines.

Register-failure rollback is currently enforced only by the application-level contract suite: registration has no production failure mode in this slice, so no scenario fabricates one. It gains a genuine production seam when registration becomes fallible (Slice 2 environment membership).

Slice 2 note: the final-member kill claim is minted inside infrastructure's `remove_member_and_finalize` rather than the application use case, because the claim must be granted atomically with the domain registry's membership removal; explicit `kill_container` remains an application use case and interface adapters stay decode/delegate/encode.
