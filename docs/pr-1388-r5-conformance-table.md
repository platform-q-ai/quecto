# PR #1388 round-5 conformance verification table

Verification branch: `feature/1369-container-backed-spawn`.

| # | Verification evidence |
|---|---|
| 1 | `quecto-agentic-harness/src/infrastructure/tools/spawn.rs` has no `ContainerLaunchContext`, `prepare_container_launch`, or `build_container_exec_command` references; production launch is selected through `AgentLaunchBackend` in `src/application/agent_launch_backend.rs`. |
| 2 | `ScriptManagedContainerLaunchBackend::prepare_launch` builds final child argv and passes `-- child_binary child_args` to `create` for NEW and to `exec` for EXISTING. Container BDD `container_spawn_launch.feature` passes. |
| 3 | `ParentEndpoint::{DirectUds, Proxy}` is typed in `agent_launch_backend.rs`; spawn records `socket_mode` into protocol/TUI structs. |
| 4 | `subagent_monitor.rs` calls `apply_container_inspect` on socket close before `notify_child_exited`; container lifecycle BDD covers one EOF post-mortem path. |
| 5 | Spawn registers script output in `ContainerRegistry`; protocol structs carry retained endpoint/workspace/status metadata. Registry BDD passes. |
| 6 | Existing kill-container unit tests pass, including script failure surfacing and member stop signalling. |
| 7 | Reference scripts validate workspace containment before destructive cleanup; core rejects unsafe repo strings. |
| 8 | `scripts/container-runtime/*.sh` and harness copies emit JSON via Python `json.dumps` with values from argv/env, not raw JSON printf. |
| 9 | Repro doc identifies #1310 Docker harness mechanics and docs now label generic scripts as host-local reference, not Docker provisioning. |
| 10 | TUI/protocol structs include `socket_mode`; container TUI BDD scenarios pass for solo/group/details. |
| 11 | `rg '@pending' quecto-agentic-harness/tests/features/container_spawn*.feature` is empty; container BDD passes. |
| 12 | `docs/container-runtimes.md` documents config, selection, argv/env, JSON, liveness, and host-local reference scripts. |

Commands run:

```text
cargo fmt --all
cargo check -p quecto-agentic-harness -p quecto-tui
cargo test -p quecto-agentic-harness --lib --no-fail-fast
cd quecto-agentic-harness && cargo test --features test-support --test bdd -- --input 'tests/features/container_spawn*.feature'
```
