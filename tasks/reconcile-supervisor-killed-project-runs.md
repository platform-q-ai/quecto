<!-- quecto-task schema=1 -->
# Reconcile project run state after a supervisor kills its coordinator

| Field | Value |
|---|---|
| Id | reconcile-supervisor-killed-project-runs |
| Kind | chore |
| Status | draft |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-25T11:48:06Z |
| Updated | 2026-10-25T11:48:06Z |

## Description
Observed twice during board-2705-b13-backup delivery: get_state could not find the coordinator, get_containers reported a supervisor-killed retained environment with zero members, but the project run remained running indefinitely until manually ended. Run1 started08:18, worker death events09:35:45, manually stopped11:09; run2 started11:15, confirmed supervisor-killed before11:45, manually stopped11:45. Actual cause/timing of supervisor termination is not established; do not assume user messages caused it. Project end_run without live swarm report exported items as todo despite primary worker-board completed-task evidence. Investigate lifecycle reconciliation and durable progress export so a dead coordinator cannot leave an apparently running project task, while preserving unpushed work. Evidence: /home/shane-pocket/.quecto/artifacts/b13-backup-recovery/handoff.md and v3/verifier-focused-red2-binding.md plus verified recovery-through-a185.bundle/log. First UUID5cf7907a-9a02-455a-af7a-fdfb06e0aebe/C157; second8f00e374-6835-48ad-9e3a-a410a96d7e15/C158; both environments removed only after verified external preservation. Any fix requires focused lifecycle regression tests and independent review; do not modify protected playbooks. This is separate follow-up, not a new board MVP delivery prerequisite.
