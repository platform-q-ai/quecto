<!-- quecto-task schema=1 -->
# Rehearse board-service cut-over on a fresh copied board and deliver the two-host runbook

| Field | Value |
|---|---|
| Id | board-2705-cutover-rehearsal |
| Kind | task |
| Status | ready |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-25T22:33:03Z |
| Updated | 2026-10-25T22:39:57Z |

## Description
Operator-authorized operational rehearsal and markdown runbook, not production cut-over or feature delivery. On edinburgh create a fresh temporary GitHub clone of platform-q-ai/quecto and fetch quecto/board read-only, record exact master/board SHAs. Build current master into scratch only, never ~/.local/bin. Derive and exercise actual current CLI: full-history board-server import; export dry-run round-trip compare; serve only on explicit scratch UDS/state; CLI list/show reads through that socket; backup and isolated restore/read verification; fence/push only to an owned scratch bare remote populated from the copied board. Allowlist every output/state/socket/Git write target under owned scratch; disable real remote pushes before feature commands. No live clones, board server, board imports/fences/exports/backups, real remote writes, deploys/installations/units, relay/state/admission/credentials/env dumps/global config/protected-playbook changes. Latest user permits only temporary scratch server/socket explicitly; prior blanket no-server rule is superseded solely for this rehearsal. B8 remains operator-owned and currently PR2754 in review; use actual master only, never its branch or unmerged code. Time each step; retain exact argv/exits, surprises/errors, captured input hash manifests, comparisons and cleanup ownership evidence externally. Deliver a markdown cut-over runbook with exact ordered edinburgh/eccup commands for drain, scratch-validated quecto upgrade on both hosts, import, fence, server startup, board=server project switches, relay/APK deploy, verification and rollback; DOCUMENT commands only, execute no live step or install/deploy. Keep post-go-live enhancements/follower/failover out of MVP. Give full runbook and rehearsal findings to operator and phone file outbox. Max2 building members globally incloperator/max1 ours/jobs1/no CPU pin/core-dump0/hard per-probe timeouts/cleanup. Substantial analysis in isolated swarms; actual host rehearsal must really be on edinburgh in its own temp clone. No source fixes/commits/PRs unless separately authorized; report unsupported commands/surprises honestly instead of implementing or waiving them. Fresh independent validation of runbook and revision-bound evidence before final report.
