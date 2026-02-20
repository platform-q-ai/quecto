---
description: Run precommit checks, commit, push, create PR, and dispatch reviewer agents
agent: build
---

Run the full ship pipeline for the current branch. Execute each step in order and stop if any step fails. Create a new TodoList to do this:

## Step 1: Precommit checks

Run the following commands from the repo root, in order. If any fails, fix the issues and re-run until all pass. Do not proceed until all are green.

1. `cargo fmt --check` — verify formatting. If it fails, run `cargo fmt` to fix, then re-check.
2. `cargo clippy -- -D warnings` — lint with all warnings as errors. Fix any issues and re-run.
3. `cargo test --lib` — run all unit tests.
4. `cargo test --test bdd` — run all BDD integration tests.

## Step 2: Commit

Look at all staged and unstaged changes with `git status` and `git diff`. Draft a concise commit message that follows conventional commit style (e.g. `feat:`, `fix:`, `refactor:`, `test:`, `docs:`, `chore:`). Check `git log --oneline -10` for recent examples. Stage all relevant changes and commit. Do not commit files that contain secrets (.env, credentials.json, etc).

## Step 3: Push

Push the current branch to the remote. If no upstream is set, push with `-u` to set tracking.

## Step 4: Create or update PR

Check if a PR already exists for this branch using `gh pr view`. If one exists, skip creation and use the existing PR number. If not, create a new PR using `gh pr create` targeting the `master` base branch. Write a clear summary with bullet points describing the changes. Return the PR URL.

## Step 5: Dispatch reviewer agents

Once the PR exists, dispatch all four reviewer subagents in parallel to review it:

1. `@architecture-reviewer` — Review the PR for architectural soundness, system design, modularity, and upstream compatibility.
2. `@security-reviewer` — Review the PR for security vulnerabilities, input validation, auth flaws, and data exposure risks.
3. `@performance-reviewer` — Review the PR for performance regressions, memory leaks, unbounded growth, and hot path efficiency.
4. `@documentation-updater` — Review PR changes and update README.md and AGENTS.md to reflect new features, commands, tools, agents, or configuration changes.

Each agent should submit a formal GitHub PR review with inline comments. Pass each agent the PR number and repo (`$ARGUMENTS` if provided, otherwise detect from current branch).

## Step 6: Address reviewer comments

After all three reviewers have finished, fetch the inline comments from the PR:

```
gh api repos/{owner}/{repo}/pulls/{number}/comments
```

For each comment:

1. Read the comment body and severity tag (e.g. `[arch]`, `[critical]`, `[leak]`, `[nit]`).
2. If the comment is actionable and correct, fix the issue in the code.
3. If the comment is a `[nit]` or informational (`[info]`), skip it unless the fix is trivial.
4. If you disagree with a comment, leave a reply explaining why.
5. Once reviewed, mark ALL comments as resolved. Branch will not merge if you dont.
6. Merge.

After making fixes, re-run the precommit checks from Step 1 to verify nothing is broken. Stage, commit, and push the fixes.

Then resolve all addressed comments by sending a `PATCH` to each comment with the reply noting the fix, or by replying on the PR thread. Do not resolve comments you chose not to fix — leave those for human review.

## Final output

Print a summary with:

- Commit hash and message
- PR URL
- Status of each reviewer dispatch (started/failed)
- Number of reviewer comments: total, fixed, skipped, disputed
