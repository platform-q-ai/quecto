---
description: Implement feature with ultra-fast local quality gates and pre-PR reviews
agent: build
---

`$ARGUMENTS`

## Goals

- Keep BDD-first development discipline
- Use local parallel test gates (25-way BDD sharding)
- Run reviewer agents locally before PR creation
- Push only after code + tests + local reviews are green

## 0) Start

1. Create/update a TodoList for this workflow.
2. Ensure you are on a feature branch (create one if needed).
3. Identify the target `.feature` file.

## 1) BDD build cycle (feature implementation)

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

## 2) Fast local quality gate (parallelized)

Run these checks, fixing failures immediately:

1. `cargo fmt --check` (run `cargo fmt` then re-check if needed)
2. `cargo clippy -- -D warnings`
3. `cargo test --lib`
4. `cargo test --test architecture`

Then run full BDD using 25-way shards.

### 2a) Full non-real-LLM BDD (25 shards)

```bash
for i in $(seq 0 24); do
  (timeout 12m env QUECTO_BDD_SHARD_INDEX=$i QUECTO_BDD_SHARD_TOTAL=25 cargo test --test bdd) &
done
wait
```

### 2b) Full real-LLM BDD (25 shards, only when required)

Requires `OPENAI_API_KEY` and paid API usage.

```bash
for i in $(seq 0 24); do
  (timeout 12m env QUECTO_REAL_LLM=1 QUECTO_TAG=real-llm QUECTO_BDD_SHARD_INDEX=$i QUECTO_BDD_SHARD_TOTAL=25 cargo test --test bdd) &
done
wait
```

Use `QUECTO_TAG=real-llm-smoke` for quicker paid smoke runs.

## 3) Local pre-PR reviewer pass (no GitHub round-trip)

Run reviewer agents locally against branch diff (`master...HEAD`) and collect findings:

1. architecture-reviewer
2. security-reviewer
3. performance-reviewer
4. documentation-updater

Apply actionable findings in one batch. Re-run Section 2 afterward.

## 4) Commit only when green

1. Inspect: `git status`, `git diff`, `git log --oneline -10`
2. Stage relevant files only (never secrets)
3. Commit with concise conventional message (`feat:`, `fix:`, `refactor:`, etc.)

## 5) Push + PR after local review is done

1. Push branch (`git push -u origin <branch>` if first push)
2. Create PR (`gh pr create`) or update existing PR
3. Include concise summary and note that full local gates passed

## 6) Post-push policy

- Prefer minimal GitHub-side checks if local gates are trusted
- Keep at least one lightweight remote safety check (recommended) to catch environment drift

## Final output

Report:

- Feature implemented and scenario count
- Files changed (step defs + production)
- Local gate results (fmt/clippy/lib/architecture/non-real BDD/real-LLM BDD)
- Reviewer findings summary (fixed/skipped)
- Commit hash/message
- PR URL (if pushed)
