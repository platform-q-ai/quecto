---
description: Run BDD dev workflow for a feature
agent: build
---

Run the BDD development cycle for a feature. The feature to work on is specified in `$ARGUMENTS` (e.g. `sandbox_hardening`). If no argument is given, look for any `@pending` feature file in `tests/features/` and ask which one to work on.

Consult `bdd-plan.md` in the repo root for the implementation order and per-feature details (step defs, unit tests, production files, dependencies).

## The Cycle

Execute each step in order. Do not skip steps. Create a TodoList to track progress.

### 1. Promote `@pending` to `@wip`

Change the feature-level tag from `@pending` (or `@done`) to `@wip` in the `.feature` file. Leave individual scenarios tagged `@pending` if they should be excluded from this cycle — the runner skips `@pending` scenarios even within a `@wip` feature.

### 2. Run BDD — expect FAIL (skipped steps)

```
cargo test --test bdd
```

This should fail because step definitions don't exist yet. The runner uses `.fail_on_skipped()` so undefined steps cause failures. Confirm the failure is from missing steps, not a parse error. Note: the runner includes both `@wip` and `@done` features, so all existing `@done` features will also run as a regression check.

### 3. Write step definitions in `tests/bdd.rs`

Add `#[given]`, `#[when]`, and `#[then]` functions to `tests/bdd.rs`. Important rules:

- Step defs go in `tests/bdd.rs` — all steps are in one file, organized by feature sections.
- The World struct is `QuectoWorld`. Add new fields to it as needed.
- For docstrings, use `step: &gherkin::Step` (not `cucumber::Step<World>`).
- Gherkin docstrings must use plain `"""` without language hints — the hint text gets included in the parsed content.
- Gherkin table `|` pipes conflict with shell metacharacters in test data — cover those edge cases in unit tests only.
- For wiremock `MockServer`, do NOT store it in the World directly (causes silent crashes). Leak it with `std::mem::forget()` and store only the URI string.
- Async traits returning futures must use `Pin<Box<dyn Future + Send + '_>>` to be dyn-compatible.

Run `cargo test --test bdd` again — it should still fail because production code doesn't exist yet.

### 4. Write unit tests — expect FAIL (red)

Add `#[test]` or `#[tokio::test]` functions in the relevant `src/` module's `#[cfg(test)] mod tests` block. These tests should cover the production logic that will make the BDD scenarios pass. Run:

```
cargo test --lib
```

This should fail because the production code isn't implemented yet.

### 5. Implement production code

Write the minimal production code to make both unit tests and BDD scenarios pass. Follow clean architecture:

- `domain/` — pure types + traits, only `thiserror` dependency
- `application/` — use cases, depends only on `domain/`
- `infrastructure/` — adapters (serde, reqwest, tokio, filesystem)
- `interface/` — CLI

### 6. Unit tests — expect PASS (green)

```
cargo test --lib
```

All unit tests must pass. Fix any failures before proceeding.

### 7. BDD tests — expect PASS (green)

```
cargo test --test bdd
```

All `@wip` scenarios (excluding `@pending` ones) must pass. Fix any failures.

### 8. Quality checks

Run formatting and lint checks:

```
cargo fmt --check
cargo clippy -- -D warnings
```

Fix any issues. If `cargo fmt --check` fails, run `cargo fmt` and verify.

### 9. Refactor

Clean up the code if needed. Re-run `cargo test --lib && cargo test --test bdd` after any refactor to confirm nothing broke.

### 10. Promote `@wip` to `@done`

Change the feature-level tag from `@wip` to `@done`. Run BDD one final time to confirm the runner now skips this feature (output should show 0 features/scenarios/steps unless other `@wip` features exist).

## Final output

Print a summary with:

- Feature name and number of scenarios implemented
- Number of unit tests added
- Production files created or modified
- All test results: `cargo test --lib` count, `cargo test --test bdd` count
- Any `@pending` scenarios left for future work
