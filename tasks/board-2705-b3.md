<!-- quecto-task schema=1 -->
# B3: Full-history import and rollback export

| Field | Value |
|---|---|
| Id | board-2705-b3 |
| Kind | task |
| Status | review |
| Parent | board-service-mvp-2705 |
| Accepted plan | 4 |
| Claimed by | shane-pocket (`shane@platformq.co.uk`) |
| Claimed since | 2026-10-25T01:50:20Z |
| Claim expires | 2026-10-25T03:50:20Z |
| Created | 2026-10-23T16:47:22Z |
| Updated | 2026-10-25T02:45:20Z |

## Description
Next authorized MVP deliverable after B2 merged PR2706 at774c78f1a7ce72efe59dc343266f4f063b86cab3 (reviewed7aa1/harness0.107.440). One complete bounded linear-history import plus exact original-history rollback and valid deterministic Git continuation for every later native SQL revision. Proper inward transfer ports, ImportBoardHistory/ExportBoardHistory application use cases, normalized SQLite history/epochs, sole common Reader, composition and additive offline board-server import/export CLI. Authoritative planning package /home/shane-pocket/.quecto/artifacts/b3-readonly-planning/; actual merged B2/B4/T1/T2/T3 interfaces must be reconciled before new RED tests. Operator B4 PR2719 root/serve/status/time already merged, snapshot deferred B11; B5-lite operator-owned/inreview2721, blocking UDS/TCP client belongs later B7. No listener/client/token duplication. Parent adopts known-scope epoch classification, explicit finite bounds and required schema3/epoch-head expectation-version updates preserving original case/assertion meaning. Forecast975–1420 production additions+deletions under accepted epic1500max; tests/fixtures separately reported, not proven ceiling. Scratch-only106+ and12-commit fixtures; no live import/export/fence/cut-over/deploy/server/unit installation or start. Fresh RED and fresh delivery separate; original B2 proofs preserved, no new test-infrastructure cycles for deferred Low2723/2724/2725.

## Depends on
| Task |
|---|
| board-2705-b2 |
