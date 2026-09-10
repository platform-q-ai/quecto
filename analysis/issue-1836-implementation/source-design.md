# Source design and route resolution (#1836)

Revision inspected: `bbcd01d32da78d22b29771bc9142ef22fd38583b`.

## Current ownership map

- `domain/environment_registry.rs`: session authority for committed environments, monotonic/non-reused `CN` refs, records/status/member state, ref/name resolution, and exclusive kill/inspect claims. `entries()` is the current list query.
- `application/environment_control.rs`: mixes the list query (`get_containers`) with kill orchestration. Kill correctly resolves, checks retained kill capability, atomically claims, snapshots under claim, delegates effects, then commits stopped or cleanup-failed.
- `infrastructure/tools/environment_kill.rs`: owns effects only—cascade termination of members and retained kill process execution. It must not acquire/settle lifecycle claims.
- `infrastructure/tools/agent_cmd_containers.rs`: `*` session route adapter and JSON encoder. Listing currently delegates to `EnvironmentControlUseCase::get_containers()`.
- `infrastructure/extensions/native.rs`: composes one shared environment registry into spawn and environment control, preserving session inventory.

## Required clean cut

1. Add `application/environments.rs` as the dedicated list-environments query/use case over `EnvironmentRegistry`.
2. Move listing out of `EnvironmentControlUseCase`; keep kill use case/port and all claim settlement unchanged.
3. Inject the listing query separately into `AgentCmdTool`; route `get_containers` to it and `kill_container` to kill control. Update native composition and test fixtures.
4. Remove obsolete list method and any temporary aliases/imports/exports; retain domain registry `entries()` unless the issue explicitly names it obsolete—the application query needs an inward repository read.
5. Keep JSON byte semantics at the adapter: `{ "containers": [...] }`; fields `ref,name,status,workspace,repository,environment_uuid,members,metadata,last_error`; all committed statuses included and deterministic `BTreeMap` ref order. Kill JSON remains `killed`, capped `agents`, optional `omitted_agents`.
6. Keep `agent_id:"*"` as the parent-local/session route for both commands. Do not route listing through child UDS or derive it from subagent inventory: that would lose empty, stopped and cleanup-failed environments. The existing pre-dispatch recognizes container commands before generic agent-id validation; update the split dependencies without changing this external route.

## Lifecycle invariant

Listing is read-only. Kill ordering remains: resolve/check capability → `begin_kill` → re-read member snapshot → terminate member subtrees/run retained kill → `complete_kill` or `fail_kill`. The `Killing` state prevents per-member finalization from double-owning teardown. No listing refactor may move registry mutation into the adapter or infrastructure port.

## Focused proof

- Application query test lists running plus stopped records directly from the authoritative registry, including stable order/snapshot behavior.
- Environment-control tests prove kill exactly once, concurrent refusal, failure/retry, and kill-less no-op semantics after list removal.
- Adapter tests prove listing JSON is unchanged and each command fails when its independently injected use case is absent.
- Composition test proves spawn, list query and kill control share the same registry.
- Architecture test forbids registry/claim/kill-port orchestration in interface adapters and forbids the obsolete list method/alias.
- Cleanup search: obsolete `EnvironmentControlUseCase::get_containers`, old listing alias/import/export, and old constructor/wiring call sites have zero matches; `get_containers` remains only as the external command name/docs/tests.
