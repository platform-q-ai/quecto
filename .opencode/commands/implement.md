---
description: Implement feature with ultra-fast local quality gates and pre-PR reviews
agent: build
---

**ALWAYS ADD THE FULL BDD CYCLE RED - GREEN - REFACTOR PROCESS TO YOUR TODO LIST**

`$ARGUMENTS`

## Goals

- Keep BDD-first development discipline
- Use local parallel test gates (24-way BDD sharding)
- Run reviewer agents locally before PR creation
- Push only after code + tests + local reviews are green

BDD build cycle (feature implementation)

1. Promote feature tag to `@wip` (from `@pending` or `@done`).
2. Run targeted BDD first (expect fail if steps/code missing).
3. Implement step definitions in `tests/bdd/` modules and world state in `tests/bdd/main.rs` if needed.
4. Re-run targeted BDD (still red is fine if production logic missing).
5. Write unit tests for production logic (expect fail/red).
6. Implement production code (minimal, clean-architecture compliant).
7. Run unit tests (green).
8. Run targeted BDD (green).
9. Refactor if needed; re-run unit + targeted BDD.
10. Promote feature tag to `@done`.
11. Create PR.
12. Deploy Local Reviewers.
13. Fix relevant comments and mark all as resolved.

ALWAYS USE: Fast local quality gate (parallelized)

Run these checks, fixing failures immediately:

1. `cargo fmt --check` (run `cargo fmt` then re-check if needed)
2. `cargo clippy -- -D warnings`
3. `cargo test --lib`
4. `cargo test --test architecture`

Then run full BDD using 24-way shards.

### 2a) Full non-real-LLM BDD (24 shards)

```bash
for i in $(seq 0 23); do
  (timeout 12m env QUECTO_BDD_SHARD_INDEX=$i QUECTO_BDD_SHARD_TOTAL=24 cargo test --test bdd) &
done
wait
```

### 2b) Full real-LLM BDD (24 shards, only when required)

Requires `OPENAI_API_KEY` and paid API usage.

```bash
for i in $(seq 0 23); do
  (timeout 12m env QUECTO_REAL_LLM=1 QUECTO_TAG=real-llm QUECTO_BDD_SHARD_INDEX=$i QUECTO_BDD_SHARD_TOTAL=24 cargo test --test bdd) &
done
wait
```

Use `QUECTO_TAG=real-llm-smoke` for quicker paid smoke runs.

Run reviewer agents locally against branch diff (`master...HEAD`) and collect findings:

1. architecture-reviewer
2. security-reviewer
3. performance-reviewer
4. documentation-updater

## Final output

Report:

- Feature implemented and scenario count
- Files changed (step defs + production)
- Local gate results (fmt/clippy/lib/architecture/non-real BDD/real-LLM BDD)
- Reviewer findings summary (fixed/skipped)
- Commit hash/message
- PR URL (if pushed)

**ALWAYS ADD THE FULL BDD CYCLE RED - GREEN - REFACTOR PROCESS TO YOUR TODO LIST**