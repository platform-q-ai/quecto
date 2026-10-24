<!-- quecto-task schema=1 -->
# Fix swarm bridge privacy test false positive on public IDs containing PID sentinel

| Field | Value |
|---|---|
| Id | swarm-bridge-public-id-privacy-sentinel |
| Kind | chore |
| Status | draft |
| Created | 2026-10-24T05:25:52Z |
| Updated | 2026-10-24T05:25:52Z |

## Description
Existing swarm_bridge_host_tests::the_host_reads_the_board_projection_without_secrets rejects the fixed private PID sentinel "4242" anywhere in serialized public projection, including legitimate random public run IDs. CI PR2706 head dd01f3dd165f2581d858b23284bc4961ef0291f3 run40065295678/job120787535903:13906passed/1failed; publicrunid5251f4242b6942eca814a8c07e1039c3 triggered "4242 crossed" at swarm_bridge_host_tests.rs218. No PID field was exposed in the printed projection. Replace this accidental substring restriction with affirmative projection-field/privacy assertions while preserving token/reservation redaction coverage. A deterministic valid public ID containing4242 must be accepted, and actual private PID fields/tokens/reservations must still be excluded. Primary evidence: /home/shane-pocket/.quecto/artifacts/b2-master-refresh/parent-workspace-tests-failure.log and [CI failure](https://github.com/platform-q-ai/quecto/actions/runs/40065295678/job/120787535903). Independent follow-up outside SQLite MVP behavior; verify targeted existing regression plus deterministic public-ID scenario.
