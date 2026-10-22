<!-- quecto-task schema=1 -->
# Move CLI policy and IO into application boundaries (#2634)

| Field | Value |
|---|---|
| Id | issue-2634-cli-policy |
| Kind | epic |
| Status | ready |
| Created | 2026-10-22T00:00:42Z |
| Updated | 2026-10-22T00:00:42Z |

## Description
Deliver GitHub issue platform-q-ai/quecto#2634 as independently mergeable PRs to master: reuse environment listing policy; relocate forgotten-record kill decisions and runtime construction; relocate run-end wait policy; move setup path IO behind setup ports; remove repeated overlay validation. Follow Clean Architecture, affirmative guards, red-first tests and fresh independent reviews. Human merges only; label merge-requested only after independent reviews are clean. Target at most 500 changed lines per PR, maximum 900 absent human exception. Only one building swarm member at a time. Never modify human checkout /home/swq/Documents/github/quecto, admission configuration/broker, global git configuration, or inspect credentials/environment variables. Stop at human escalation checkpoints.
