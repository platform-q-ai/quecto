# #1714 — adversarial review loop 1

Target: uncommitted worktree /tmp/quecto-empty-stream-diagnostics vs HEAD 3e4d5826eb8a8fea7b64d3be7d7a0d91e55e564d. No PR exists; this is the single retained review instead of a GitHub submission. Scope notes and preserved diagnosis read. No source edits, commits, branch changes, containers, or live provider tests.

## Verified actionable findings

1. **P2 — OpenAI terminal SSE error is recorded as Dropped with no terminal event.** `quecto-agentic-harness/src/infrastructure/providers/attempt_transport.rs:435–450,498–517,570–581` (line locations may shift during ongoing edits). HTTP 200 with `data: {"error":{"code":"rate_limit_exceeded"}}` emits terminal Error, but terminal classification reads the absent `type`; the OpenAI error branch marks failure only. The pump returns on Done without EOF and the forwarder only marks failure, so the published record has `terminal_event=None`, `termination=Dropped`. Set the protocol-specific terminal diagnostic fields for this recognized error and test the incremental path. Independent REFUTE verifier CONFIRMED: no later setter repairs this normal exit. Existing exact-payload protocol test checks failure/terminal control booleans, not diagnostic terminal fields.

2. **P2 — Supported Codex reasoning can be reported as no thinking/unknown event.** `quecto-agentic-harness/src/infrastructure/providers/attempt_transport.rs:432–434,451–472`; supported parser: `codex_sse_handler.rs:65–82`. A newline-terminated `data: {"type":"response.reasoning.summary_text.delta","delta":"x"}` followed by a read error emits ThinkingDelta but retains `generated_thinking=false` and increments `unknown_events`. Observer handles only the underscore alias; its known-event allowlist excludes both aliases. Done-response reconciliation does not help this error exit. Recognize the parser's supported reasoning vocabulary and add this deterministic partial-stream failure fixture. Independent REFUTE verifier CONFIRMED by full setter/forwarder tracing.

## Workflow / validation evidence

- Built-in adversarial-review selected; target fetched locally (explicit no-PR adaptation).
- Three parallel read-only narrow finders completed: privacy/bounds NO FINDINGS; race/retry NO FINDINGS on updated ordering; test-falsifiability supplied finding 1. Parent inspection supplied finding 2.
- Separate read-only REFUTE verifiers for both surviving candidates: both CONFIRMED. No speculative findings retained.
- First-terminal suppression, terminal-before-observation ordering, and Done-only output-presence candidates were excluded after concurrent transport fixes and passing tests.
- Offline latest transport suite: 11 passed (including first-terminal suppression, publish-before-terminal and Done-only presence). Application stream suite: 3 passed. Existing inference_admission_sse_observer integration suite: 15 passed. `empty` lib filter: 171 passed. Broad/mistargeted filters returning zero are not counted as coverage.
- Transient RED failures and one transient syntax error occurred while transport edits were in progress; latest transport rerun passed. This is a changing, not frozen, code revision and not a whole-tree green certification.
- Evidence: /tmp/1714-review-final-evidence.txt (final line excerpts and hashes), /tmp/1714-review-last-test.txt, /tmp/1714-review-extra-tests.txt, /tmp/1714-review-final-tests.txt. Review should be rechecked in second loop against finished diff.

Disposition: two verified P2 findings; report retained locally and sent to parent, no GitHub write.
