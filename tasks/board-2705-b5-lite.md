<!-- quecto-task schema=1 -->
# B5-lite: Tailscale TCP with per-device tokens

| Field | Value |
|---|---|
| Id | board-2705-b5-lite |
| Kind | task |
| Status | draft |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-23T16:48:11Z |
| Updated | 2026-10-23T16:48:11Z |

## Description
MVP stage6 epic #2705. TCP listener bound only configured Tailscale IP, per-device hashed token persistence and UDS-only token add/revoke/list; affirmative role/route authorization and immediate revoke. Principals distinct from self-asserted identities. Reuse bounded UDS server parser, no hand-rolled spike parser. Members table/replication deferred. One complete PR baseline, target300–500 hard900 including tests/version. No installs/start or live token/state changes, no credential reads/env output; all parent safety restrictions apply.

## Depends on
| Task |
|---|
| board-2705-b7 |
