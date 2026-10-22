<!-- quecto-task schema=1 -->
# Move setup host-path IO behind application port

| Field | Value |
|---|---|
| Id | 2634-setup-paths |
| Kind | task |
| Status | draft |
| Parent | issue-2634-cli-policy |
| Created | 2026-10-22T00:12:51Z |
| Updated | 2026-10-22T00:12:51Z |

## Description
Issue #2634 concern 4. Move direct and indirect setup canonicalization/current-directory IO behind host-path port and infrastructure adapter, with composed configuration selection. Preserve explicit-config refusal before IO, project precedence, symlink identity, fallback/errors and root/absolute defenses. Baseline ef17b93e0d5058e07fdd3e54f05e6c86f3e31243. Target 450–500 changed lines including tests; provisional upper 650. Re-estimate and split production-consumed preparation from final integration if over target. Full evidence: /home/swq/.quecto/phase-artifacts/issue-2634-planning/handoff.md S4. No established baseline-compiling semantic red; explicit human decision required.
