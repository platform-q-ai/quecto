# Agent/dev quickstart

This is the single source of truth for Quecto repository agents. Prefer these commands over rediscovering crate names, test flags, GitHub calls, or hook behavior.

## Repo orientation

- Kernel crate: `quecto`, located in `quecto-agentic-harness/`.
- TUI crate: `quecto-tui`, located in `quecto-tui/`.
- Use `cargo … -p quecto` for kernel work and `cargo … -p quecto-tui` for TUI work; never `-p quecto-agentic-harness` because that is a directory name, not a crate.
- Layers are `domain`, `application`, `infrastructure`, and `interface`. Domain stays pure; application coordinates use cases; infrastructure contains adapters such as tools, extensions, providers and embedded docs; interface contains CLI/TUI/UDS-facing presentation. Architecture tests enforce these boundaries.
- Runtime tools live under `quecto-agentic-harness/src/infrastructure/tools/`; extensions under `src/infrastructure/extensions/`; providers under `src/infrastructure/providers/`; embedded agent docs live in `quecto-agentic-harness/docs/` and are served by the docs tool.

## File-size cap

- Keep source files under the 750-line cap.
- Fix oversized test-heavy modules by moving tests to a sibling file and wiring it with `#[cfg(test)] #[path = "module_tests.rs"] mod tests;`.

## GitHub and `gh` recipes

- Read an issue with comments and acceptance criteria: `gh issue view <N> --json title,body,comments`.
- Read a PR diff: `gh pr diff <PR>`.
- Read PR identity/state: `gh pr view <PR> --json id,headRefOid,state,mergeStateStatus`.
- Check CI: `gh pr checks <PR>`.
- Required checks are `Unit Tests` and `Mock LLM E2E Tests`.
- Reviewer rule from #946: pass reviewers the PR number and head SHA, never a raw diff. Reviewers fetch the diff themselves with `gh pr diff <PR>`.

### Inline review GraphQL snippet

Use `addPullRequestReview` with event `COMMENT` (or an equivalent submitted inline review API). Fetch PR id and head SHA, build comments with `path`, `line`, and `body`, and submit the review so it is not left pending:

```bash
read -r PR_ID HEAD_SHA < <(gh pr view "$PR" --json id,headRefOid --jq '[.id,.headRefOid] | @tsv')
gh api graphql -f query='mutation($pr:ID!,$sha:String!,$comments:[DraftPullRequestReviewComment!]!){
  addPullRequestReview(input:{pullRequestId:$pr,commitOID:$sha,event:COMMENT,comments:$comments}) {
    pullRequestReview { id state submittedAt }
  }
}' -F pr="$PR_ID" -F sha="$HEAD_SHA" -F comments='[{"path":"path/to/file.rs","line":123,"body":"severity: problem; concrete fix"}]'
```

## Targeted tests and BDD selection

- Kernel unit/lib test: `cargo test -p quecto --lib <name_substring>`.
- TUI unit/lib test: `cargo test -p quecto-tui --lib <name_substring>`.
- Plain lib tests need no `--features` flag.
- The `test-support` feature exists only for test-only public APIs and clippy; it is not needed for ordinary `--lib` tests.
- Render-harness-driven TUI tests using `TuiHarness` are behind `--features test-harness` and run through the integration `bdd` target, not plain `--lib`.
- Tests live in sibling `*_tests.rs` files when split out from implementation modules.
- BDD feature files live in `tests/features/*.feature`; step definitions live in `tests/bdd/`.
- Current BDD selection is by tag plus sharding (for example `@mock-llm` / non-real shards). There is no scenario-name filter; do not invent one.
- RED verification: run only the new or changed targeted test to confirm it fails before implementation.

## Lint, coverage and gates

- Format: `cargo fmt`.
- Strict kernel clippy: `cargo clippy -p quecto --all-targets -- -D warnings`.
- Manual coverage needs both LLVM env vars, otherwise agents can hit llvm-tools discovery failures:
  `LLVM_COV=$(command -v llvm-cov) LLVM_PROFDATA=$(command -v llvm-profdata) cargo llvm-cov --lib -p quecto --summary-only`.
- The region coverage threshold is 87%, and the full gate checks both `quecto` and `quecto-tui`.
- Validate without pushing by running the canonical full gate script: `scripts/pre-push.sh`. Never use `--no-verify`.

## Hook behavior

- `git commit` pre-commit does NOT run unit/BDD tests. It runs the quality gate (750-line cap, work markers, lint-bypass and `unsafe` detection), the BDD-quality gate, `cargo fmt --check`, strict clippy, and fast guard tests (architecture/contracts/repo_docs/workflow_config). Commit is structure + lint + fast guards.
- `git push` pre-push runs the full new+old suite: lib and integration tests, 24-shard non-real BDD, coverage, mock-LLM e2e, `cargo machete`, and `cargo deny`. Push may take minutes and is not hung.
- After GREEN, do a quick targeted test only. Do not manually re-run the whole suite before commit. Trust push: pre-push runs all new + old tests, BDD, and coverage.
- A bypassed gate does not count. Never use `--no-verify`.

## Gotchas

- `find.feature` has a load-flaky `Nested .gitignore … in git repo` scenario under the parallel shard wave. Re-run the failed shard before treating it as real.
- Do not run parallel feature workflows in one tree; worktree/core.bare state and generated artifacts are fragile under concurrent feature runs.
- Version bumps must propagate through `Cargo.toml`, `README.md`, and `tests/features/repo_docs.feature`.
- Secrets live in `config.json`; never echo them in logs or prompts.
