Runtime agent GREEN integration in progress. Using existing leaf API with_attempt_admission(Arc<dyn AttemptAdmission>). Owning provider_runtime.rs, new provider_runtime_admission.rs; adding optional admission-aware factory helpers in providers/mod.rs (preserve every existing signature); adding infrastructure/mod.rs export. No other leaf edits. Stable alternate factory captures alias-bound gate before all OAuth closures including kernel-owned Codex-aware OpenAI rebuild (bypasses legacy admission-unaware callback only in explicit enabled ingress). Test seam swaps Proposal/Candidate/CurrentRuntimeIngress delegation for public API; assertions untouched.

Production ingress/helper wiring compiles. Runtime16 run /tmp/p2-runtime-green.log currently 3 PASS / 13 FAIL: unknown-alias rejection now passes; twelve restart rejection/store-retention checks pass and reach final exact shared queue placement, which still fails 0 != 1 pending leaf GREEN. No assertion edits. Imported AttemptAdmission via application::ports only. Unit run /tmp/p2-runtime-unit.log temporarily blocked by unused FallbackCooldown import in domain/inference_admission_policy.rs (parent concurrently editing). No local authority service added.
Runtime existing unit regression now GREEN: cargo test -p quecto-agentic-harness --lib infrastructure::provider_runtime => 40 passed, 0 failed (/tmp/p2-runtime-unit.log). Shared domain compile issue resolved by parent.

RUNTIME16 GREEN after parent leaf integration: cargo test -p quecto-agentic-harness --test inference_admission_runtime => 16 passed, 0 failed, 0 ignored (/tmp/p2-runtime-green.log). Test assertions unchanged; Proposal/Candidate now aliases for public production types and fixture builds only authority-bound gates, production factory executes validation/composition. Added two context tests in owned sibling for unknown initial aliases/missing gates and missing constructed-slot binding. Latest lib test temporarily blocked by parent's Codex refactor removing SseHandler/SseLineOutcome test imports (20 lib errors in codex_cov_tests.rs); not edited leaf scope.

Bounded actual production mutations (restored before handoff):
- Ignored validate_candidate error => capacity, queue-capacity, terminal-capacity targeted runtime tests fail at expect_err with wrongly published generation 3. /tmp/p2-runtime-mutation-reload.log, exit 101, 3 failed.
- Made production bound_gate return None => explicit_aliases_survive fails at shared queued count 0 != 1. /tmp/p2-runtime-mutation-gate.log, exit 101, 1 failed.
- Restored files and reran runtime16 => 16 passed, zero ignored, /tmp/p2-runtime-green-restored.log.

No CLI/default activation, local authority/scheduler, or ad hoc service creation. Capabilities injected by alias; P3 host may supply authenticated scope-bound ports. Context compares policy values exhaustively (all GroupPolicy/AdmissionConfig fields), aliases and bindings; no secrets, serialization, Debug/pointer comparison. Stable gates threaded into OpenAI Chat/Responses, Codex, Anthropic factories before Arc erasure, all registry refresh closures, built-in Anthropic refresh, kernel-owned enabled OpenAI refresh. Disabled path retains existing caller-supplied OpenAI callback. OpenAI callback refresh-specific grant/send assertions not added here; do not claim runtime16 proves the actual 401 path. The context trusts that supplied opaque AttemptAdmission ports belong to the stated aliases/policy; it cannot inspect their authority, which is explicitly P3 host composition responsibility.

## Follow-up: actual runtime OAuth and context mutation evidence

Extracted inline sibling unit module into provider_runtime_admission_tests.rs (architecture-compatible #[path] mod tests only in production). Restored lib compile now GREEN: 42 provider_runtime + provider_runtime_admission unit tests, /tmp/p2-runtime-unit-restored.log. Two context tests each have production mutation sensitivity:
- bypass initial alias validation => unknown alias assertion fails, /tmp/p2-runtime-unit-mutation-alias.log;
- remove missing capability check => missing bound-port assertion fails, /tmp/p2-runtime-unit-mutation-capability.log;
- bound_gate returns None => missing constructed-slot binding test fails at unexpectedly successful composition, /tmp/p2-runtime-unit-mutation-slot.log.
All compiled/executed mutations exit101 and restored.

New tests/inference_admission_runtime_oauth.rs: 2 PASS, public actual composition + real 401 HTTP + actual production refresh callback selection. Seed credential is a JWT with old-account; refresh persists a new JWT with rotated-account. Bound slot uses stable-explicit-account, unrelated to either JWT claim. Exact two POST /codex/responses requests assert stripped retained-model, old/new Bearer tokens and old/new chatgpt-account-id. Enabled second acquire pauses on explicit semaphore barrier: same injected gate has two acquisitions, first transport finished Failure, exactly one wire send until grant; then success content and [Failure, Success]. Legacy supplied factory callback count exactly0 enabled. Disabled ingress real401 invokes caller callback exactly1, zero gate acquisitions, exact rotated wire credentials and response. No wall sleeps, just notify enqueue barrier + positive I/O safety timeout.

Actual production mutations, all restored:
- enabled OpenAI factory closure drops captured gate => enabled test fails waiting for second acquire barrier (/tmp/p2-runtime-oauth-mutation-gate.log).
- enabled factory ignores rotated token => enabled test fails on wrong rebuilt wire request (/tmp/p2-runtime-oauth-mutation-token.log).
- disabled factory retains old inner instead of invoking legacy callback => disabled test fails on second401 (/tmp/p2-runtime-oauth-mutation-disabled.log).
These are behavioral mutation failures, not compilation errors (an initial invalid callback mutation was corrected before recording final disabled log).

Restored joint runtime16+OAuth2 => 18 PASS (/tmp/p2-runtime-oauth-restored.log); final OAuth2 after last mutation => 2 PASS (/tmp/p2-runtime-oauth-final.log). This supersedes previous OpenAI actual401 limitation for these two nonincremental chat paths; no new claim for actual Anthropic/registry refresh paths or all streaming refresh surfaces.

Final quality checks: architecture46 PASS (/tmp/p2-runtime-architecture-final.log). Replaced seventh named-OpenAI helper argument with cohesive ProviderTransportContext { client, admission } (no lint suppression; existing public API unchanged). cargo clippy -p quecto-agentic-harness --lib -- -D warnings => PASS (/tmp/p2-runtime-clippy.log). Re-ran runtime16+OAuth2 after context refactor => 18 PASS (/tmp/p2-runtime-after-context.log).

Fallback-base follow-up in progress: added independent fallback_base reload case with nondefault live base37, equal-value no-op acceptance, candidate38 rejection and existing retained publication/budget/wire oracles. Not modifying GroupPolicy literals (domain agent318 owns all). Waiting on additive domain field before executing legitimate compile/run RED with equality intentionally omitting the new value, then GREEN exhaustive equality.

## Explicit fallback_base_ms runtime follow-up

Domain agent owned the new field and all literals. Runtime-only changes:
- Exhaustive GroupPolicy value comparison includes fallback_base_ms.
- Independent fallback_base_reload_rejected_retaining_nondefault_live_runtime test uses live base37, separately allocated equal-value no-op, rejected candidate38, retained generation/catalogue/runtime/credential/group/budget assertions and exact wire checks.
- Following successful retained runtime HTTP, real authority NoHint feedback verifies first fallback retains base37: now91 -> cooldown128 (not default1000 or candidate38).

Test-first behavioral RED before equality implementation: destructured new fallback field as ignored, ran new reload test: compiled/executed 1 FAIL at expect_err with wrongly published candidate generation3 (/tmp/p2-runtime-fallback-base-red.log). Implemented equality by including new field in both tuples; new test GREEN (/tmp/p2-runtime-fallback-base-green.log). No literal race edits. Full restored runtime17+OAuth2 => 19 PASS, zero ignored (/tmp/p2-runtime-fallback-base-suite.log). RED is the exact omission-of-new-policy-value mutant; no claim that compile-only exhaustiveness was behavioral evidence.
