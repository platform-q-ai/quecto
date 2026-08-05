# PR #1388 round-5 conformance reproduction

Baseline reviewed: `feature/1369-container-backed-spawn` at `3af353a322b50e5bb66a996524de00eb91d255a2` (recorded before local fixes). Issue #1369 body confirms #1310 supplied Docker/local-TUI mechanics: UDS harness wiring, host/socket exposure, repo clone/update with SSH, and host/user identity mapping. Existing reusable mechanics in this repository are `scripts/docker-harness-local-tui.sh`, `scripts/docker-harness-entrypoint.sh`, `docker-compose.harness.yml`, `docker/quecto-harness.Dockerfile`, and docs in `docs/docker-harness.md`. The generic `scripts/container-runtime/*` scripts must not claim Docker behavior unless they adapt those mechanics.

## Valid reproduced gaps

1. Spawn runtime neutrality failed: at baseline `quecto-agentic-harness/src/infrastructure/tools/spawn.rs` stored `ContainerLaunchContext` and called `prepare_container_launch` / `build_container_exec_command` directly. This put script/runtime preparation in the tool instead of behind `AgentLaunchBackend`.

2. NEW launch semantics failed: baseline `spawn.rs` prepared container before building child CLI, so `create` could not receive final child argv; after CLI construction, spawn built an exec command. Thus new environments were created then joined, instead of create starting the child. EXISTING only mutated registry during prepare and relied on later exec.

3. `socket_proxy` failed: baseline `container_launch.rs` parsed/stored `socket_proxy`, but spawn collapsed endpoint to a filesystem socket path and readiness/monitor/routing used only direct UDS paths. A typed parent endpoint is required.

4. Persistent liveness failed: baseline readiness used bounded connect polling and lifetime/death came from wrapper process `wait`, with inspect invoked from the reaper path. There was no single persistent idle child connection whose EOF/reset drove inspect, status update, await completion, and TUI broadcast.

5. ContainerRegistry authority failed: baseline registry existed, but `get_containers` / `kill_container` reconstructed containers by scanning subagent entries in `uds_query.rs`. Unknown/stale refs and stopped/empty environments could not be represented authoritatively.

6. Shared cleanup failed: baseline reaper cleanup and explicit `kill_container` independently selected owners and ran kill scripts; failures were swallowed in cleanup helpers, leaving state potentially untruthful.

7. Containment boundary failed: Rust passed script-reported `QUECTO_WORKSPACE_PATH` into destructive cleanup without a core trusted-root validation boundary. Reference kill had partial containment checks only.

8. Robust JSON failed: baseline reference create/exec interpolated shell values into Python source and inspect/kill used raw printf JSON. Quotes/control chars in values could corrupt output or source.

9. #1310 mechanics were not honestly adapted: baseline reference scripts were host-local tempdir wrappers while metadata claimed `docker-reference`. Valid correction is either adapt existing docker-harness mechanics or document host-local examples honestly; no new Docker image/provisioning design is in scope.

10. TUI spec incomplete: issue requires solo dim container ref, grouped rows when two or more agents share an environment, nested/suppressed root children, selectable details including ref/name/status/repo/branch/runtime/workspace/socket mode, and metadata surviving roster refresh. Existing evidence shows metadata is roster-derived rather than authoritative.

11. Pending/synthetic coverage remained: `quecto-agentic-harness/tests/features/container_spawn.feature` had multiple `@pending` scenarios; `container_spawn_lifecycle_steps.rs` fabricated JSON/world flags for liveness instead of driving production seams.

12. Docs incomplete/misleading: baseline docs described Docker-oriented reference scripts while scripts were host-local and omitted concrete config/selection/argv/env/JSON/liveness/reference walkthrough details.

## Reviewer overreach / scope boundaries

- Issue explicitly forbids designing Docker images/provisioning inside Quecto production code. Rust should stay runtime-neutral and call scripts. Existing #1310 Docker mechanics can be documented or adapted by scripts, not embedded as Docker policy in Rust.
- String command parsing may remain an argv-safety boundary if validated consistently; the core conformance failure is production spawn bypassing/duplicating launch-backend responsibilities and unsafe script JSON, not necessarily requiring a new public config schema in this PR.
