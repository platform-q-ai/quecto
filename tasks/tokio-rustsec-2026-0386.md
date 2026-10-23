<!-- quecto-task schema=1 -->
# Resolve Tokio RUSTSEC-2026-0386 blocking dependency CI

| Field | Value |
|---|---|
| Id | tokio-rustsec-2026-0386 |
| Kind | task |
| Status | archived |
| Created | 2026-10-23T15:04:43Z |
| Updated | 2026-10-23T15:44:47Z |

## Description
Dependency Policy on layout PR #2648 exact head f2dde116a4ed91d8f2d5f25efb8ad6337dd03d00 fails cargo deny for Tokio1.49.0, RUSTSEC-2026-0386: AsyncFd constructors violate I/O safety. [Primary CI run](https://github.com/platform-q-ai/quecto/actions/runs/39974024137) reports error[unsound] at Cargo.lock. Separately scope a reviewed dependency remediation on master, preserve behavior and supported APIs, verify targeted checks and required full CI. No advisory suppression or dependency change is authorized by the moves-only layout landing scope. Human authorization is required before implementation. Once fixed on master, refresh #2648 by master merge commit, recompute version and verify exact new head independently before serial layout landing continues.
