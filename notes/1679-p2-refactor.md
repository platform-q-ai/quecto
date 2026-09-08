# P2 refactor/deleted-invariant audit

- Actual leaf attempt gate below router/refresh/retry. Public LlmProvider methods
  and borrowed requests unchanged; optional fields default None. No top-level
  provider decorator changes name/as_any. All disabled branches retain old sends.
- Extracted CodexSseHandler moves code to sibling module rather than exceeding750
  line cap. Existing SSE limits/error parsing stay handler-owned;469 provider tests
  and disabled4 characterization confirm relocation behavior.
- Enabled assembled Codex uses owned SSE pump/collector instead of buffering full
  body and applying feedback at EOF. Typed advice visible at event receipt while
  response body held. Existing handler resource limits preserved in both paths.
- Shared OwnedTransport explicitly destroys request/response future before finish;
  success/failure acknowledged only after teardown. Reverse-order production
  extraction mutant killed two trace tests. Receiver closure/deadline/cancel while
  HTTP/body/channel-blocked selects against actual owner, not an outer receiver.
- One internal bounded channel1 observes existing StreamEvent errors without
  rewriting handlers; downstream original channel64 remains bounded. No unbounded
  forwarding or new retry owner. Error after transport release can await consumer
  without retaining admission occupancy.
- Runtime factory optional ingress retains externally supplied stable alias-bound
  capabilities through pre-erasure constructors and OAuth rebuild; rejects changed
  policy/map/enablement before publication. Normal factory remains disabled.
- Parsed advice dependency httpdate replaces handwritten calendar arithmetic;
  only existing locked package promoted. All three HTTP wire formats covered.
- Shared fallback is domain group state with injected scalar randomness. NoHint
  receipt idempotency precedes increment, success resets count but not cooldown.
- New application interfaces reexported only through ports for infrastructure;
  contracts reuse actual transport integration rather than fake-only checks.

No speculative local polling authority registered. P3 authority implementation,
IPC and persistence remain out of scope. Draft sketches/evidence notes are not
production activation. Review must continue to challenge terminal errors,
backpressure, abort races and independent alias identity rather than infer host
correctness from in-process fixtures.
Superseded failing stubs/sketches moved from untracked notes/1679-p2-drafts to
/tmp/1679-p2-archived-drafts before delivery; historical earlier note references
are chronology, not current implementation or committed dead code.
Review-driven refinements: separate Profile captures only preexisting vendor ×
surface diagnostics/read behavior. Assembly keeps full response bytes exactly as
legacy parser requires, with bounded independent receipt line observer; observer
must share provider terminal/event interpretation (review fix ongoing). No formatter
may erase billing codes needed by existing initiation classifier. Introduced
OpenAI error behavior must retain typed metadata while leaving disabled branch.
ProtocolObserver review refinement must represent vendor enum, not anthropic bool
plus universal JSON heuristic. This keeps OpenAI envelope semantics distinct from
Responses typed event dispatch; terminal/error interpretation follows respective
parser contracts. Future providers must not silently inherit another wire grammar.
Hook marker audit cleanup: ReplyPlan and GroupSends group cohesive fixture inputs,
reducing helpers to5args; assertions/trace/control flow unchanged. Removed all new
allow attributes, exported reusable fixture modules within test crates.83targeted
GREEN plus strict clippy (/tmp/admission-fixture-cleanup-{tests,clippy}.log).
