<!-- quecto-task schema=1 -->
# Add dev-only JSON schema validation infrastructure for declaration contract

| Field | Value |
|---|---|
| Id | 2708-schema-prereq |
| Kind | task |
| Status | ready |
| Parent | issue-2708 |
| Created | 2026-10-23T23:03:44Z |
| Updated | 2026-10-23T23:03:44Z |

## Description
Independent prerequisite slice split from C1 first remediation: genuine dev-only JSON-schema validator needed for schema-aware regression; generated lockfile growth would push C1 beyond900. Add dependency/configuration contract red via actual Cargo metadata graph, not source text, then dev dependency and generated lock changes in separate green PR. No runtime activation, new agent/user wording, playbook/Jev/deploy/live-state access. Preserve C1 red history and unresolved findings for dependent fix. Target300–500/max900 with locklines reported separately. Full parent split decision /home/shane-pocket/.quecto/artifacts/issue-2708/prereq-split-decision.md; original C1 review-final-c1/HANDOFF.md and fix-c1-r1/HANDOFF.md. Separate fresh red/delivery/independent-review phases, serialmerge only exact-headreview+greenCI, finalpatchbump and merge-requested.
