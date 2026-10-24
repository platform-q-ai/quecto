<!-- quecto-task schema=1 -->
# Operator-owned B5-lite: Tailscale TCP and per-device tokens

| Field | Value |
|---|---|
| Id | board-2705-b5-lite |
| Kind | task |
| Status | draft |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-23T16:48:11Z |
| Updated | 2026-10-24T10:13:57Z |

## Description
Operator-owned B5-lite for epic2705, following merged operator B4 PR2719/version0.107.436. TCP on the configured Tailscale IP with per-device tokens; members table deferred. This agent does not start, implement or duplicate B5-lite. B7 must wait for its actual merge and consume the real TCP/auth/config interfaces. The blocking client over both UDS and TCP belongs to agent B7, not B5. B4 snapshot route is deferred to B11. Latest lane is B2>B3>B7 only after operator B5-lite lands>B8>reduced B13-lite>PocketP1P2(no deployment). Current accepted epic size policy applies, superseding historical all-in900 wording. No live token/state/credential access, installation/service/unit start/deployment/cut-over, protected playbook or relay/admission changes.

## Depends on
| Task |
|---|
| board-2705-b4-b6 |
