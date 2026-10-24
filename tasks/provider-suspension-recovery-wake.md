<!-- quecto-task schema=1 -->
# Investigate coordinator recovery wake after suspended executor and command deadline

| Field | Value |
|---|---|
| Id | provider-suspension-recovery-wake |
| Kind | task |
| Status | draft |
| Parent | issue-2708 |
| Created | 2026-10-24T23:27:27Z |
| Updated | 2026-10-24T23:27:27Z |

## Description
Separate follow-up, not part of root-only guard delivery. During Pocket PR141 R4/R5 review, executor provider request errors left idle agents with automaticTurnsSuspended=true; board messages could not resume them and coordinator playbook disallows direct worker follow_up. Parent authorized managed termination and same-slot recovery. In R5 a preserved owned cargo/test command had hard1200-second deadline23:05:24Z, last process audit22:52:42, and coordinator still yielded pending quiescence until parent received a30-minute quiet notification at23:24Z. Current process state/timeout enforcement at23:24 was not yet established: do not claim the deadline actually failed. Independently investigate source and bounded controlled fixtures: verify timeout enforcement separately from wake delivery, executor-suspension notices, ownership-safe same-slot recovery and coordinator decision routing; prevent waiting without a scheduled observation or explicit blocker. Preserve max-building limits, immutable process birth/origin checks, no unknown signals/credential/admission reads. No JeV, no live relay changes; root-only guard child-unaffected scope remains unchanged. Primary external evidence: issue-2708/review-pocket141-r5/{RECOVERY-DECISION.md,recovery/known-process-recheck-once.json,recovery/natural-check41.json,executor/}; Parent recovery receipts6dbdb938 andc4ecc0a1. Draft requires separate approved planning/delivery, no implementation authorized here.
