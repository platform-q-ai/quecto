# Issue 1714 — adversarial review loop 2

## Retained finding (1)

**P2 — Malformed Anthropic terminal payload is reported as dropped rather than terminal.** Semantic location: `quecto-agentic-harness/src/infrastructure/providers/attempt_transport.rs:488–505` (terminal assignment is at 418–461).

With admission enabled and a request trace, HTTP 200 followed by `event: error\ndata: {\n` makes the Anthropic parser dispatch a terminal Error, but the published attempt has `wire_status=200`, `parse_errors=1`, `terminal_event=None`, and `termination=Dropped`. `message_stop` with the same malformed data similarly emits Done without terminal diagnostics. The terminal event name is available and allowlisted even though the payload is malformed.

Diagnostic terminal assignment occurs only after successful JSON decoding. The malformed-JSON-compatible Anthropic branch sets the private observer terminal flag, not the diagnostic fields. The pump returns immediately on terminal dispatch (no EOF setter); forwarding marks failure only; owner destruction publishes the unchanged snapshot. Preserve the recognized terminal metadata independently of JSON payload decoding and cover both terminal names with behavioral tests.

Independent adversarial refuter: **CONFIRMED** by complete source tracing of observer, actual Anthropic parser, pump, forwarding, owner destruction, and observation publication. No live reproduction or new test was performed in this read-only review. No retry-policy regression is alleged.

## Workflow and evidence

- Built-in `adversarial-review`; three narrow read-only finders: terminal ordering/release NO FINDINGS; privacy/bounds NO FINDINGS; wire truth supplied the sole finding above. One separate read-only verifier explicitly attempted to REFUTE it and confirmed it. No speculative findings retained.
- Prior loop's nested OpenAI terminal error and dotted Codex reasoning fixes verified present; their behavioral tests pass.
- Offline tests: transport module 13 passed; application stream 3 passed; request observation 3 passed; provider error classification 22 passed. Zero-test filters are excluded from coverage. Logs: `/tmp/issue1714-loop2-tests.txt`, `/tmp/issue1714-loop2-transport-tests.txt`, `/tmp/issue1714-loop2-domain-tests.txt`. Diff whitespace check passed.
- Parent's earlier full-library/clippy gates are context, not independently rerun certification in this review.
- Read acceptance notes, `quecto-agentic-harness/docs/empty-stream-diagnostics.md`, and prior loop report.

## Target / concurrent movement caveat

Started against uncommitted worktree `/tmp/quecto-empty-stream-diagnostics` versus HEAD `3e4d5826eb8a8fea7b64d3be7d7a0d91e55e564d`. Saved tracked diff `/tmp/issue1714-loop2.diff` SHA-256 `257c7c0800eb4960f65ae1c935967ea78d0df53348a2468b0f5a3cb0866a3f3a`; newly added source was also reviewed.

An external actor committed the target during review as `c20f2808be9a0e0d2995982ccc3d1ec0a1865bc2`; its tracked baseline diff was byte-identical to the saved diff. Subsequent external commits extracted/renamed test modules, reaching `da187ceb509161f028f8224e37cd831f19066541`. Relevant production behavior remained unchanged. This review made no source edits, commits, branch changes, container/swarm operations, or provider requests.

No PR exists: this is the single retained review in place of GitHub submission; no GitHub write was made. Disposition: **one verified P2 remains**.

## Final PR handoff (supersedes earlier no-PR disposition)

PR #1716 subsequently opened. Fetched PR metadata and diff; verified OPEN at final head `da187ceb509161f028f8224e37cd831f19066541`. Base remains `3e4d5826`. Changes since c20f2808 are test extraction/module naming and notes, not the affected production path. Exactly one COMMENT review submitted, containing the confirmed P2 in its body with semantic source locations (inline position unavailable):
https://github.com/platform-q-ai/quecto/pull/1716#pullrequestreview-5155730742

GraphQL verified state COMMENTED and non-null submittedAt `2026-09-09T14:30:45Z`; response retained `/tmp/issue1714-loop2-submitted.json`. No merge or labels. Parent reports additional passing pre-push architecture/contracts/clippy and full 4323 tests before module rename; these were not independently rerun by this reviewer.
