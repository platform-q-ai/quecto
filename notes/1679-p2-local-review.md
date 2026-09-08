# P2 local precommit review (step9 ongoing)

Architecture reviewer764a: confirmed missing operator fallback base. ADR0026line45
requires positive finite configured base; GroupPolicy lacks field and policy hard-
codes1000. Counterexample required10sec/base120sec => siblings start1sec. Refutation
attempt: standalone FallbackCooldown::new has base but no group input, insufficient.
Accepted fix: explicit fallback_base_ms validated and included reload comparison;
shared-service nondefault tests RED and restart-only change test required.

Affected matrix AC5 config boundary +AC3 restart-only now includes fallback base.
Rerun architecture + semantic +cross-file reviewers after fix, before commit.
Semantic reviewa8 confirmed2: OpenAI typed SSE ignored by handler then Done, fixture
explicitly excluded OpenAI; Responses top-level type:error/code absent classifier
root support. Refutation failed tracing lines120–143 handler +classifier nested
only. Fix enabled stream errors with explicit terminal event/no Done and no replay,
add all-leaf pre/posttext tests; top-level typed-error root classifier only when
type=error, terminal billing precedence. Preserve disabled legacy path unless
canonical behavior requires separate characterized correction. Rerun semantic and
cross-file angles after RED/GREEN fixes.
Cross-file d68 findings accepted after checking old/new branches:
1 incremental HTTP error output4KiB truncation removed in helper (whole-body
   classification allowed but displayed payload must retain old bound).
2 Anthropic send raw Display now source-chain => Unknown→Network retry change;
   Codex old prefix changed. Preserve vendor surface formatting policy explicitly.
3 OpenAI/Anthropic nonstream read errors previously fail before status, helper
   unwrap_default converts truncated429 to status retry plus fallback.
4 assembled Anthropic/Codex prior body read through EOF detects truncation after
   terminal event; new pump exits early success and drops final unterminated line.
   Preserve full-body completion/parse semantics while observing SSE advice at
   receipt. Don't apply wholebody protocol to incremental surfaces.
Cancellation display-string comparison is a design smell, not independently proven
failure; replace with typed internal stopped outcome while touching helper.
Differential enabled/disabled loopback regression tests required RED before fixes.
Fallback fix verified:3 assertion RED before production validation/config consumer,
actual1002vs10002 and invalid config accepted. Added explicit field all literals
migrated preserving previous values, shared6 CargoGREEN/domain73 isolatedGREEN.
Runtime comparison includes field. No shared mutation residue. Evidence
/tmp/p2-fallback-config-{red,green,cargo-green}.log. Await reviewer rerun after
other changes complete.
Runtime base37/no-op/rejected38 test: omission comparison RED wrongly publishes
3, restored equalityGREEN; real subsequent NoHint91→128 retains37. Runtime17+
OAuth2 GREEN, /tmp/p2-runtime-fallback-base-{red,suite}.log.
Root classifier fix12 tests:4 behavioralRED,8 negative/precedence mutant kills,
final helper64+typed11GREEN; root participates only type:error, billing across root/
nested wins, prose ignored. /tmp/admission-feedback-root-{red,green,mutations}.log.
Actual wire top-level test still leaf owner verifying before rerun review.
Compatibility23 actual differential RED captured: disabled characterization passes,
enabled same URL/frame fails exact parity for all23, no fixture failures or timeout.
Includes raw/classifier/status/retryability and all assembled fields. Always-permit
observer one grant/finish,2 physical POSTs; unproxied loopback. /tmp/admission-error-red.log.
Helper repair authorized after RED, tests unchanged expected baseline.
OpenAI SSE actual5 nowGREEN including disabled old Done characterization, enabled
pre/posttext oneerror/noreplay/noDone, terminal billing/untyped no cooldown;
/tmp/p2-openai-parent-green.log. Compatibility23 previously GREEN owner run; parent
rerun temporarily compile-blocked test-owner oracle extraction (not production
regression), waiting stable test helper.
After profile/assembled repair parent actual attempts65/fallback5/OAuth2 and
architecture46GREEN. Boundaries preserve transport ownership despite assembly
compatibility reinstatement. /tmp/p2-review-leaf-parent-green.log,
/tmp/p2-review-architecture-green.log. Awaiting final compatibility oracle controls
and wire top-level Responses regression before fresh reviewer cycle.
Parent current compatibility26GREEN (23actual+3oracle), Responses root4GREEN,
strict alltargets clippyGREEN, fmt applied. /tmp/p2-review-compat-parent-green.log,
/tmp/p2-responses-root-parent-green.log,/tmp/p2-review-clippy.log. All three affected
local review angles rerunning complete current diff+new files, not only fixes.
Compatibility owner final92 counterexamples at same live oracle entrypoints:
parity16, disabled characterization64, fixture12; valid controls accepted then one
observed fact changed, every assertion rejects. /tmp/admission-error-sensitivity.log
and /tmp/admission-error-sensitivity-ids.txt. Final26parallelGREEN. Setup socket/
join/parse timeout guards are not semantic claims; no fake enforcement coverage.
Postfix parent full lib4035GREEN, tagged BDD6/30GREEN. Root wire4 initiallyGREEN
needs dedicated production mutant +member oracle evidence before review completion;
owner tasked to add proof rather infer from sibling nested tests.
Architecture rerun764: prior resolved; no new material architecture/reuse findings.
Reconstructed config/binding/transport-profile/typed cancel; fallback6,reload1,
architecture46pass, full5diff+new files inspected. Await semantic/regression reruns.
Semantic rerun new finding accepted: introduced OpenAI error text loses structured
code/type so rate_limit_error opaque Unknown/no retry, billing code+rate-limit prose
retryable. Refutation tracing existing classifier confirms billing requires JSON
code/type. Need preserve structured fields when rendering new enabled OpenAI error,
carry typed status/classification to existing initiation owner, test preoutput
retry and terminal billing, no new retry owner. Matrix P2-08 includes typed error
classification ×preoutput owner rather than leaf error-only success.
Crossfile rerun prior resolved, new material observer-terminal mismatch accepted:
assembled parser ignores trailing error after terminal while independent receipt
observer still throttles/fails. Continue HTTP EOF validation but stop interpreted
advice at provider semantic terminal (Responses completed/[DONE], Anthropic event
message_stop). Also match Anthropic event dispatch rather than arbitrary data JSON.
Add terminal-success×trailing-throttle actual test before fix; re-review crossfile.
Disabled P0 characterization4/transport2 final recheckGREEN after profile fixes,
/tmp/p2-review-disabled-final.log. Notes missing fixture README probe was harmless
path error; actual peer.py fixture unchanged apart from explicit base literals in
other tests. No environment blocker remains.
New actual initiation3:2RED opaque type no retry/billing unexpectedly retries,
posttext compatible1 pass; typed JSON formatter fix targetedGREEN. Observer5 all
RED (trailing semantic terminal/ignored Anthropic event), protocol-state fix in
progress. /tmp/p2-openai-retry-owner-{red,green}.log /tmp/p2-observer-red.log.
Parent initiation/observer restoredGREEN including protocol-state observer member
counterexamples; /tmp/p2-review-semantic-owner-green.log. Rerunning semantic and
crossfile review after second material corrections, architecture clean previous
rerun unchanged inward binding/config. No review finding dismissed solely by tests.
Current parent workspace strict clippyGREEN, architecture46/contracts242GREEN,
fmt/diffcheck pass. /tmp/p2-review-workspace-clippy-final.log and
/tmp/p2-review-contracts-final.log. Extra wire regressions separate targets, covered
in prior latest green runs and will remain hook-visible full workflow verification.
Leaf final proof read: isolated copied-production root operand removal kills all4
actual wire receipt count/kind; restored copiedGREEN no shared residue. Nine shared
wire predicates counterexamples cover request/text/errors/Done/grants/receipt/
finish/abandonment. Retry-owner/observer member oracles added. Final leaf targets
65+26+16+5+4+6+5+2+6GREEN; provider481GREEN. Logs /tmp/p2-all-audited-leaf-green.log,
/tmp/p2-root-actual-wire-{mutated-red,restored-green}.log. Await clean rerun verdicts.
Semantic final reruna8 CLEAN: prior findings resolved no new defensible material;
independent11 integration suites154GREEN, all5diffs+newfiles+canonical/matrix read.
Parent final full lib4035GREEN /tmp/p2-local-final-lib.log. Crossfile final pending.
Crossfile final still residual accepted: non-Anthropic observer conflates OpenAI
error-envelope with Responses extension.error. Actual Codex parser ignores unknown
type, observer throttles/marks terminal hides later genuine error. Split exact
vendor dispatch and test APIkey/OAuth×assembled/incremental unknown→completion and
unknown→genuine throttle. No broad "JSON contains error" inference allowed.
Hook preflight discovered #[allow] markers in new shared fixture modules despite
strict clippy passing. Removed dead-code suppressions by exposing test fixture
modules through integration/BDD root; these are test crates only. Argument-suppression
cleanup owner replacing helpers with coherent inputs. Current no-deadcode-allow
clippyGREEN /tmp/p2-no-allow-clippy.log. No lint policy relaxation.
Responses residual8 actualRED then observer15GREEN: vendor enum now exact dispatch,
unknown error object cannot throttle or hide subsequent valid event. OpenAI cannot
inherit Responses completed terminal. /tmp/p2-extension-{red,green}.log. Final
semantic/crossfile reruns requested; architecture binding/config unchanged.
Semantic affected-angle final CLEAN after vendor dispatch:4suites50GREEN including
observer15, all5diff+newfile/matrix checked. No new material. Parent observer15GREEN
and wrapperquality/fmt/diff pass. Crossfile final verdict pending retrieval.
Crossfile final CLEAN prior resolved/no new defensible material,158targetedGREEN
including observer15. All5diffs+untracked source/test inspected; real loopback
oracles supplemented not replaced by counterexamples. All required local angles
now clean; no outstanding material finding or mutation residue.
