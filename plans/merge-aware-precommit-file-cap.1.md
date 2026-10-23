<!-- quecto-plan schema=1 -->
# Plan 1 of merge-aware-precommit-file-cap

| Field | Value |
|---|---|
| Task | merge-aware-precommit-file-cap |
| Version | 1 |
| State | draft |
| Proposed by | shane-pocket (`shane@platformq.co.uk`) |
| Created | 2026-10-23T18:38:24Z |
| Updated | 2026-10-23T18:38:24Z |

## Approach
Add a narrow positive exemption only to the existing new-Rust-file750-line predicate: stage-0 regular blob must equal a same-path regular blob in a valid MERGE_HEAD commit. Validate the complete parent set before allowing any match; absent, malformed, noncommit, unknown, nonregular or changed candidates retain the limit. Preserve executable-bit-insensitive content identity, linked-worktree git-path resolution, quoted literal paths, existing worktree line counting, and every other hook check. First create actual-hook temporary-Git regression tests in existing hook_scripts_worktree architecture harness, prove intended RED and baseline rejection controls, commit tests only and hand off verified base-to-RED bundle without pushing. Fresh delivery implements and validates, final own version-only master+1, foreground push/open PR/merge-requested full CI. Fresh independent exact-head review follows PR publication, then parent authorized merge after green checks and resolved threads. Finally consume the hook fix in #2650 without bypass.

## Task items
| Task item | Title |
|---|---|
| red | Actual-hook RED regression and preserved new-file controls |
| fix | Minimal affirmative merge-parent content exemption |
| publish | Validate, version and publish standalone fix |
| review | Independent review and authorized master landing |

## Task item dependencies
| Task item | Depends on |
|---|---|
| fix | red |
| publish | fix |
| review | publish |

## Criteria
| Task item | Criterion |
|---|---|
| red | Temporary real Git/index/MERGE_HEAD objects and real hook/formatting, no source-text assertions or command stubs |
| red | Identical merge-imported751-line staged file fails baseline for exact size diagnostic; genuinely new751 rejects and750 passes |
| red | Test-only commit and verified base-to-RED bundle/SHA256 plus commands/exits/counts durable and accessible; no failing push |
| fix | Same-path stage-0 regular blob equals regular merge-parent blob; complete valid parent set before accepting any match |
| fix | Missing/invalid/noncommit/unmerged/nonregular/different content retains limit; exact literal quoted paths including spaces |
| fix | Size predicate only; normal line counting, branch/fmt/unsafe/BDD rules preserved |
| publish | Focused actual-hook and installer/activation tests green, fmt/lint/applicable gates with counts |
| publish | Target500/hard900 including tests; own final version-only master+1, security lock content preserved |
| publish | Foreground push timeout1500 minimum, open PR, verify head, merge-requested exact-head full CI |
| review | Fresh independent PR review at exact head validates RED unchanged, fail-closed cases and preserved rules |
| review | All required CI green, threads resolved and master/head fresh before parent merge --merge |
| review | Subsequent #2650 integration uses normal updated hook, no bypass |
