# Issue #1708 validation

## Contract

The TUI locally projects dated admission cooldown snapshots with monotonic time for the master footer and child panel. Local expiry is rendered as `cooldown elapsed`; it does not change lifecycle/admission state or claim authoritative availability.

## TDD evidence

- RED: `.quecto/issue-1708-red.log` — the new paused-clock master/child regression expected `1s` after two elapsed seconds but observed the static `3s` snapshot.
- GREEN focused: `.quecto/issue-1708-green-focused.log`.
- Admission suite: `.quecto/issue-1708-admission-suite.log` — 18 passed.
- Strict TUI Clippy: `.quecto/issue-1708-clippy.log` — passed.
- TUI library suite: 2,184 passed; one pre-existing branch-refresh test fails reproducibly in isolation because this swarm checkout changes branches (`.quecto/issue-1708-tui-lib-rerun.log`, `.quecto/issue-1708-unrelated-rerun.log`).

The authoritative DTO and revision are not mutated during rendering. Projection uses saturating monotonic arithmetic and only the allowlisted `until` state with a present duration. Unknown, unavailable, unsupported, and missing-duration states are not fabricated into dated countdowns.
