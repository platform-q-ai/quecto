# Issue #1836 test and architecture plan

Baseline revision: `bbcd01d32da78d22b29771bc9142ef22fd38583b`.

## Scope and invariants

Move environment inventory from `EnvironmentControlUseCase::get_containers` to the synchronous application query `application/environments/list_environments.rs`. The query must read the session's authoritative `EnvironmentRegistry`; preserve registry iteration order and every `EnvironmentRecord` field; include running, empty, killing, stopped, and cleanup-failed records; include detached/zero-member snapshots; and retain registry poison-recovery behavior. The public allowlisted `agent_cmd` tuple `("*", "get_containers")` delegates to this query and preserves current JSON. Kill orchestration and lifecycle ownership remain in `EnvironmentControlUseCase`.

Remove the obsolete listing method, alias/export/imports, and update every consumer without unrelated behavior changes.

## Red → green → refactor

### Red

1. Add application query tests first in `src/application/environments/list_environments_tests.rs`:
   - empty registry returns an empty list;
   - committed records across lifecycle states are returned in authoritative iteration order;
   - complete records (identity, name, workspace, repository, script, retained argv, members, status, metadata, error) are unchanged;
   - detached/zero-member records remain visible;
   - returned values are snapshots (subsequent registry changes do not mutate an earlier result);
   - registry poison recovery remains observable through the query, using the existing registry poison-test idiom.
2. Move/remove the old `get_containers` use-case test and add an architecture test requiring the new query while rejecting the obsolete method/alias.
3. Update adapter tests with a spy/query seam where practical: only the affirmative exact command/target combination reaches listing; rejected/malformed commands make zero calls; successful output retains the complete JSON shape and order.
4. Run the focused command and retain the expected compiler/assertion failure before implementation:

```bash
cargo test -p quecto-agentic-harness application::environments::list_environments --lib
cargo test -p quecto-agentic-harness --test architecture list_environments_query_has_one_application_home -- --exact
```

### Green

Implement only enough to satisfy the tests: create the application query, inject/wire it from the same session registry, migrate the adapter, and remove obsolete listing API surface.

```bash
cargo test -p quecto-agentic-harness application::environments::list_environments --lib
cargo test -p quecto-agentic-harness infrastructure::tools::agent_cmd_containers_tests --lib
cargo test -p quecto-agentic-harness --test architecture
```

### Refactor and behavior protection

Keep the query synchronous and side-effect free. Keep lifecycle transitions, kill claims, member termination, retained kill invocation, and finalization outside it. Preserve these BDD behaviors:

- `tests/features/script_managed_environments_slice2.feature`
- `tests/features/script_managed_liveness_slice3.feature`
- `tests/features/script_managed_runtime_slice5.feature`

Run full BDD unless the runner's supported filter syntax is first confirmed:

```bash
cargo test -p quecto-agentic-harness --test bdd
```

## Required validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --features quecto-agentic-harness/test-support -- \
  -D warnings -W clippy::cognitive_complexity -W clippy::too_many_arguments -W clippy::too_many_lines
cargo test --workspace --lib --bins
cargo test -p quecto-agentic-harness --no-fail-fast --test architecture --test contracts
cargo test -p quecto-agentic-harness --no-fail-fast --test bdd
```

## Cleanup and architecture evidence

Use explicit allowlisted expected locations when interpreting results. Command/protocol string occurrences of `get_containers` remain valid; obsolete Rust method/alias/import/export occurrences do not.

```bash
rg -n 'get_containers\s*\(|EnvironmentControlUseCase::get_containers|environment_control_app|application::environment_control' \
  quecto-agentic-harness/src quecto-agentic-harness/tests
rg -n 'list_environments|ListEnvironments' quecto-agentic-harness/src
rg -n 'EnvironmentRegistry' \
  quecto-agentic-harness/src/infrastructure/tools/agent_cmd_containers.rs \
  quecto-agentic-harness/src/interface/cli/uds_query.rs
```

Expected final evidence:

- zero obsolete listing method, compatibility alias, import, or export hits;
- `ListEnvironments` occurs only in the application query/tests and explicit composition/adapter delegation locations;
- infrastructure/interface handlers do not directly own or query `EnvironmentRegistry`;
- exact JSON inventory, ordering, lifecycle visibility, and `agent_id: "*"` routing are covered;
- red failure and subsequent green command logs are retained at the implemented revision.
