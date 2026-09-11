# #1924 verification — coordinator loss keeps the environment; member logs captured

Branch: `worktree-agent-a6da5764ef7a3a958` (off `origin/master` at 742f05fc).

## What changed

### Cascade fix (main work)

- **Domain** (`quecto-agentic-harness/src/domain/environment_finalization.rs`):
  `HostedSwarmRun` + `SwarmRunObservation` (`NoStore` / `Run` / `Unreadable`)
  and the pure policy `retains_environment(mode, &observation)`: an `Exit`
  that empties an environment hosting a resumable run (created, `running` or
  `paused`) — or whose store exists but cannot be read — withholds the
  retained kill. `EnvironmentFinalizationPort` gains two defaulted methods
  (`observe_hosted_swarm_run`, `record_lost_coordinator`); the use case calls
  them only in `Exit` mode after the kill claim is minted. Rollbacks,
  `ParentKill` (kill_container, `agent_cmd kill`, master shutdown) are
  untouched.
- **Domain registry** (`environment_registry.rs`): new `EnvironmentStatus::
  Retained` (label `retained`), `retain(claim, reason)` records the reason
  under `metadata.retained`; `begin_kill` accepts `Retained` (explicit
  `kill_container` still closes it); `resolve_joinable`/`add_member` accept
  it and a join revives the record to `Running`.
- **Lost-harness rule reused** (Python): the master quarantines the
  coordinator through the existing `_quarantine` → `_end_by_loss` path
  (paused holding `failed`). `_status` now also reports the coordinator.
  `swarm_policy.resume_blockers` gains a `lost_coordinator` blocker;
  `Coordination._receipt` adds `resume_blockers` to every control receipt and
  `resume` uses the same computation (a resume with a lost coordinator is
  refused, naming it). `Transaction.lost_coordinator()` derives the loss from
  the events ledger (`scope_unknown` newer than `activated`), no JSON1 needed.
- **Infrastructure**: `swarm_bridge::HostedStore` (host-side, membership-free
  store handle; `store_rpc`/`bootstrap_source` factored out of
  `SwarmContext`), retrying a contended read 4x. `ScriptEnvironmentFinalization
  Port` in `subagent_cleanup.rs` locates the store through the create-result
  `metadata.checkout` key. `RunControlReceipt.resume_blockers` decoded and
  surfaced on the `swarm_control` UDS reply.
- **Adapter**: `scripts/container-runtime/docker/create.sh` reports
  `metadata.checkout` (= `$child_cwd`, the `QUECTO_SWARM_CHECKOUT` it hands
  members).
- **TUI**: `retained` ranked like `empty` in environment status aggregation.

### Log capture

- `create.sh` passes `-e "RUST_LOG=${RUST_LOG:-info}"` before `HOME`, with the
  comment explaining why (identical to the user's uncommitted master edit).
- Docs: `docs/container-runtimes.md` (adapter section: RUST_LOG, `journalctl
  --user CONTAINER_NAME=quecto-env-<id>`, `kill.log`; new "Coordinator loss
  keeps a swarm's environment" section; `metadata.checkout` in the create
  contract; `retained` status), `quecto-agentic-harness/docs/swarm.md`
  (operator runbook: loss behaviour + log reading), and the embedded agent
  manual `docs/docs-tool-embeds/swarm.md` (`resume_blockers`, retained env).
- Version bumped `quecto-agentic-harness` 0.107.5 → 0.107.6 (docs changed).

## Tests added

- `src/application/environment_finalization_tests.rs`: policy matrix
  (running/paused retain; placeholder/terminal/no-store do not; unreadable
  retains; every non-Exit mode never retains), coordinator exit withholds
  kill + retains + records loss, store refusal still retains, unreadable
  store retains without a loss record, plain member exit unchanged,
  bootstrap placeholder unchanged, launch rollback unchanged, join revives +
  explicit kill claim closes.
- `src/infrastructure/tools/subagent_monitor_coordinator_loss_tests.rs`
  (through `notify_child_exited` with `ConnectionClosed`, a real sqlite store
  and a logging kill script): coordinator loss → no kill argv, record
  `Retained`, run `paused`/`failed`, `resume_blockers` names
  `'coordinator'`, `resume_external` refused; placeholder-run container and
  store-less environment still run the kill.
- `tests/container_runtime_docs.rs`: create.sh run through a recording fake
  CLI — `RUST_LOG=info` present in the run argv, host `RUST_LOG` override
  wins, `metadata.checkout` equals the `QUECTO_SWARM_CHECKOUT` env.
- BDD `tests/features/script_managed_liveness_slice3.feature`, scenario
  "A coordinator connection loss retains the environment of a running
  swarm" (steps in `tests/bdd/spawn_liveness_steps.rs`): real script-managed
  spawn, child killed behind Quecto's back, 1 inspect / 0 kills, listing
  `retained` with reason, hosted run paused holding `failed` with the
  blocker, then `kill_container` runs exactly 1 kill → `stopped`.

## Commands run (all from the worktree root unless noted)

| Command | Result |
| --- | --- |
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets --features quecto-agentic-harness/test-support -- -D warnings -W clippy::cognitive_complexity -W clippy::too_many_arguments -W clippy::too_many_lines` | exit 0, no warnings |
| `cargo test -p quecto-agentic-harness --lib` | 4383 passed (includes the 19 new finalization + coordinator-loss tests) |
| `cargo test --test architecture` (harness) | 49 passed |
| `cargo test --test contracts` (harness) | 285 passed |
| `cargo test -p quecto-agentic-harness --test container_runtime_docs` | 11 passed (3 new) |
| `cargo test -p quecto-agentic-harness --test repo_docs` | 14 passed |
| `QUECTO_TAG=container-liveness cargo test --features test-support --test bdd` | 14 scenarios / 113 steps passed (new scenario included) |
| `QUECTO_TAG=swarm ... --test bdd` | 48 scenarios / 216 steps passed |
| `QUECTO_TAG=swarm-supervision ... --test bdd` | 2 / 7 passed |
| `QUECTO_TAG=container-env` / `container-spawn` / `container-runtime` `... --test bdd` | 15/116, 26/141, 12/103 passed |
| `cargo test -p quecto-tui --lib environment </dev/null` | 29 passed |

## Adversarial self-review findings (fixed)

- (a) *Retained kill still runs for a coordinator loss*: an unreadable
  (contended/corrupt) store originally mapped to `None` → ordinary kill.
  Fixed: observation is a three-way `SwarmRunObservation`; `Unreadable`
  retains, and the host read retries 4x before giving up. Remaining
  deliberate teardown paths: `agent_cmd kill` of the coordinator agent
  (`ParentKill`) and the master's own shutdown — documented, unchanged.
- (b) *Leak — retained environment never closable*: `begin_kill` accepts
  `Retained` (BDD proves `kill_container` closes it); a join revives it to
  `Running`. A retained environment outlives a master session that never
  closes it; documented with the manual removal path (`kill.sh`, `podman rm
  -f quecto-<id>` + state dir).
- (c) *Plain non-swarm container child behaviour*: every container carries
  a bootstrap placeholder store (deadline 0, `setup`) → `resumable()` false
  → ordinary kill; store-less (host-local reference scripts, no
  `metadata.checkout`) → `NoStore` → ordinary kill. Both covered by tests.
- Python `lost_coordinator` initially used `json_extract` (SQLite JSON1);
  replaced with a Python-side parse so container images with older SQLite
  builds are unaffected.

## Not done / follow-ups

- Relaunching the coordinator against the surviving store on `resume`
  (issue's second slice): a resume is refused naming the coordinator; a
  manual spawn into the retained environment (`mode: existing`) revives the
  record, but re-activating the same swarm member identity is not wired.
- The master's own SIGTERM (`ParentKill` at process shutdown) still tears
  down a running swarm's container — out of scope, noted in docs.
- The Docker/Podman adapter itself is not exercised in CI (no runtime);
  `create.sh` is characterised through a recording fake CLI.
