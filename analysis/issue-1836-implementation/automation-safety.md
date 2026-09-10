# Automation safety inspection

Inspected at repository revision `bbcd01d3` without changing GitHub state.

## Commands and observed evidence

```bash
gh label list --limit 200 --json name,description,color | jq -c '.[] | select(.name=="claimed" or .name=="merge-requested" or .name=="release-binaries")'
```

Observed:
- `merge-requested`: “Explicitly request authoritative CI for the current PR revision”.
- `claimed`: “Supervised agent claim; comments record owner, scope and handoff. Not an atomic lock.”
- `release-binaries`: “Publish binary release artifacts after a merged master PR”.

```bash
gh api repos/platform-q-ai/quecto/branches/master/protection | jq '{required_status_checks,required_pull_request_reviews,enforce_admins,required_conversation_resolution,allow_force_pushes,allow_deletions,required_linear_history}'
```

Observed strict required checks:
1. Static Quality
2. Workspace Tests
3. Non-Real BDD
4. TUI BDD
5. Mock LLM E2E
6. Coverage
7. Dependency Policy
8. Review Threads Resolved

Protection is strict/up-to-date, enforced for admins, and requires conversation resolution. It requires zero approving reviews; code-owner and last-push approval are disabled. Force pushes and deletion are disabled.

```bash
gh api repos/platform-q-ai/quecto | jq '{allow_auto_merge,allow_merge_commit,allow_squash_merge,allow_rebase_merge,delete_branch_on_merge,default_branch}'
```

Observed `allow_auto_merge: true`: GitHub permits an authorized actor to enable it. This is a residual operator risk, not an automation path in this repository.

```bash
gh api repos/platform-q-ai/quecto/rulesets
```

Observed `[]`; branch protection above is authoritative.

## Workflow conclusions

- `.github/workflows/ci.yml` triggers on PR `labeled` events targeting master, but every merge job has an affirmative condition for label `merge-requested`. Applying it runs Authoritative Merge CI. Concurrency is per PR and `cancel-in-progress: true`, so another label-triggered run for that PR cancels the earlier run.
- `.github/workflows/reset-merge-requested.yml` triggers on PR `synchronize` and affirmatively removes `merge-requested` if present. Therefore a pushed fix invalidates the prior request; reapply the label only after the new head is final, then watch checks on that exact `headRefOid`.
- No workflow reacts to `claimed`; it is cooperative metadata only.
- Repository searches found no automation that calls `gh pr merge`, GraphQL `enablePullRequestAutoMerge`, or otherwise enables auto-merge. `quecto-agentic-harness/docs/workflow.md:337-363` explicitly says “do not merge”, “Do not merge or set auto-merge”, and guards `git merge`/`gh pr merge`.
- Consequently `merge-requested` itself cannot merge the PR. Because repository auto-merge capability is enabled, explicitly verify the target PR reports auto-merge disabled and never invoke `gh pr merge --auto` or an equivalent API mutation.
- `.github/workflows/binary-release.yml` is post-merge release automation only. A merged master PR carrying `release-binaries` may publish tags/releases; it does not merge PRs.

## Safe exact-head procedure

1. Record `gh pr view <n> --json headRefOid,autoMergeRequest,mergeStateStatus,labels` and require `autoMergeRequest == null`.
2. Apply `merge-requested` only to the final reviewed head.
3. Run `gh pr checks <n> --watch` and then re-read `headRefOid`; require it equals the recorded SHA and all eight protected contexts succeeded.
4. If any commit is pushed, allow Reset Merge Request to remove the label, finish fixes/review, reapply it, and repeat exact-head verification.
5. Leave the PR open and unmerged for human disposition.
