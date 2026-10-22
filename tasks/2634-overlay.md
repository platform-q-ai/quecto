<!-- quecto-task schema=1 -->
# Centralize overlay write eligibility in patch use case

| Field | Value |
|---|---|
| Id | 2634-overlay |
| Kind | task |
| Status | draft |
| Parent | issue-2634-cli-policy |
| Created | 2026-10-22T00:12:53Z |
| Updated | 2026-10-22T00:12:53Z |

## Description
Issue #2634 concern 5. Remove set/unset overlay write preflight duplication; map scope flags purely and present typed PatchConfiguration refusals. Preserve no-write guarantees, global explicit-config success, SharedOverlay protections, trust input helper and diagnostics. Baseline ef17b93e0d5058e07fdd3e54f05e6c86f3e31243; estimate 100–180 changed lines including tests. Full evidence: /home/swq/.quecto/phase-artifacts/issue-2634-planning/handoff.md S5. No established baseline-compiling semantic red; explicit human decision required.
