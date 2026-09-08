# P2 RED evidence (in progress)

Base c3b597e0, working branch feat/1679-p2-provider-attempts.
`cargo test -p quecto-agentic-harness --test contracts admission_feedback`
initial missing report_feedback/ThrottleFeedback compile RED, then minimal failing
port stub: 7/7 fail (six at report setup, lifecycle assertion gets Unavailable
rather than expected Conflict). /tmp/p2-receipt-red.log. This is scenario initial
RED, not yet individual assertion proof; localized mutation evidence remains
required after GREEN. No claim of completed phase/workflow.

Resumed step6: restored draft API/test registration. Parser targeted lib run:
38/38 fail at stub, /tmp/p2-red-resumed.log. Corrected unused test-only imports
so integration tests reach their intended RED rather than unrelated lint failure.
Fallback arithmetic: eight single-assertion named tests each fail at assertion
against zero-returning stub; invalid config cases fail because stub accepts them.
`cargo test -p quecto-agentic-harness --test contracts admission_fallback`:
0 passed/8 failed, /tmp/p2-fallback-red.log. Missing-module initial failure was
replaced with executable assertion RED. No production behavior implemented yet.
New @wip provider Gherkin scenarios registered. Targeted tagged BDD RED:
QUECTO_TAG=inference-admission cargo test -p quecto-agentic-harness --features
test-support --test bdd: existing3 pass/new3 fail missing step definitions.
/tmp/p2-bdd-red.log. This is discovery RED only; Then falsifiability still pending.
Typed HTTP/SSE body classifier tests: false stub fails all three positive single
assertions; temporary true mutation fails four negative assertions (billing code,
auth/client, prose-only). Restored false stub immediately. Logs /tmp/p2-typed-red.log
and /tmp/p2-typed-negative-red.log. This proves seven assertions; integration with
actual HTTP status/event receipt and partial-output handling is still pending.
Fallback equal-base/max and midpoint injected-jitter assertions added: final10/10
fail against stub. Midpoint specifies normalized u64 randomness mapped across
inclusive [base, upper] using wide arithmetic; no platform RNG in domain.
Receipt replay fencing and above-maximum absolute deadline (102 at receipt1,
max100) added independently: each fails final assertion against stub.
/tmp/p2-receipt-replay-red.log and /tmp/p2-receipt-overflow-red.log.
Continuation: repaired runtime fixture poll_once to permit dyn Future (?Sized).
Targeted runtime tests now execute: 3 fail for absent alias admission/reload
rejection, 1 disabled-compatibility passes (/tmp/p2-runtime-red.log). Earlier
compile failure not counted. Actual-leaf suite now executes all56 cases, all fail
at missing preceding grant/exact count/cancel-no-send assertions, without timeout
(/tmp/p2-attempt-resume-red.log). Later lifetime assertions remain masked by initial
missing gate; per-assertion falsifiability remains incomplete, do not mark step6.
Runtime disabled credential-rotation compatibility oracle falsified by changing
expected rotated Authorization to Bearer WRONG: intended assertion failure, then
restored exact original file and targeted test GREEN. Logs
/tmp/p2-runtime-compat-mutation.log and /tmp/p2-runtime-compat-restored.log.
This is one oracle only, not full multiassertion runtime proof.
Parser immutable receipt-wall and noncandidate401-with-valid-hint cases added;
individually fail at normalization stub (/tmp/p2-wall-red.log,
/tmp/p2-status-red.log), 40 parser cases total. Wall test pins historical receipt
so consulting live wall time cannot accidentally pass.
Additional typed envelope cases: direct SSE code positive fails false stub;
null/success text/terminal billing nested code negatives fail true mutation.
Restored stub. /tmp/p2-typed-cases-red.log and
/tmp/p2-typed-cases-negative-red.log. Eleven classification assertions total.
Past-deadline acceptance and active-cancel-before-ack receipt acceptance assertions
fail independently against stub (/tmp/p2-past-receipt-red.log,
/tmp/p2-cancel-receipt-red.log). Active cancellation remains occupied by P1 design;
feedback in that state cannot be silently dropped while transport remains owned.
Reconciled receipt suite after four boundary additions: 33 tests, 24 assertion RED,
9 expected-compatible pass with earlier mutation evidence; targeted current run
/tmp/p2-receipt-current-red.log. Step6 remains unmarked pending attempt/runtime/
feedback/BDD assertion reconciliation, not because these receipt tests are absent.
Retry exhaustion real HTTP test added: three raw requests and two existing retry
sleeps pass disabled characterization; exact grants=3 fails (actual0),
/tmp/p2-retry-red.log. Sleep occupancy/terminal/count compatibility assertions
still need independent perturbation evidence before step6 complete.
Retry test four passing compatibility assertions independently falsified by
expected-value perturbations (sleep active1, expected success, sleeps3, sends4),
each reaches intended failure then exact original restored. Logs
/tmp/p2-retry-{sleep,error,sleeps,count}-falsifiable.log. This demonstrates oracles,
not a substitute for post-GREEN production mutation release-inside-sleep.
Real HTTP401 refresh/resend added: old→fresh Authorization exactly1 each, returned
fresh content, refresh outside occupancy; retained gate must show2 grants (actual0
RED /tmp/p2-refresh-red.log). Three preceding compatibility assertions separately
perturbed active1/text wrong/rawcount3, each exit101 at intended assertion;
/tmp/p2-refresh-{active,text,count}-falsifiable.log. Original restored. Production
OAuth composition retention remains covered by separate runtime task.
Runtime review returned shared pure-oracle controls +20 counterexamples exercising
wire/budget/publication/pending/content; same helpers used by integration. Final
runtime16 cases: 14 RED/2 pass, no ignored (/tmp/p2-runtime-with-oracles-red.log).
These are preimplementation oracle sensitivity only; production mutation tests
remain mandatory after GREEN. Retry/refresh final targeted run 2 RED, clean
unproxied fixture clients (/tmp/p2-retry-final-red.log).
Fallback independent counter assertion (second group first duration10 while first
has advanced) RED stub0: /tmp/p2-fallback-isolation-red.log. Domain fallback11 total;
this isolates arithmetic object state, actual alias sharing still runtime scope.
Public AttemptAdmission/AttemptPermit contract registration reuses actual leaf and
feedback integration suites via contracts modules (also lib discovery) rather
than fake-only tests. Targeted registration smoke reaches expected grant-count
RED /tmp/p2-port-contract-red.log. Assertions are identical shared functions; no
additional behavioral claim introduced by registration.
Feedback transport follow-up complete:35 acceptance IDs H01–13/G01–12/S01–10,
all evaluated before failure set assertion;63 counterexamples detect assigned ID
and assert entrypoint panic, valid controls pass. Real target15 RED/1 oracle pass,
no timing/setup failures. Group now positive parked-acquire OR raw-send bypass
barrier, no sleep. Full matrix notes/1679-p2-feedback-red.md. Exact90s arithmetic
remains parent normalizer tests, wire30s is not a cap claim.
Parent verification reran feedback63-counterexample oracle and runtime20 oracle:
each targeted1 pass (/tmp/p2-feedback-oracle-verify.log,
/tmp/p2-runtime-oracle-verify.log). New behavior remains RED, not GREEN.
Final targeted fallback11/11 RED reconfirmed after contract registrations,
/tmp/p2-fallback-final-red.log. No full regression suite run during RED step.
Receipt clock regression standalone assertion RED (stubUnavailable vs required
TimeRegression), /tmp/p2-receipt-clock-red.log. Receipt34 total.
Parent reran all7 shared attempt oracle-counterexample checks: pass,
/tmp/p2-attempt-oracles-verify.log. Actual58 provider cases still RED; synthetic
oracle passing evidence is explicitly not provider completion.
BDD definitions now executable: tagged run3 existing pass/3 provider scenarios RED,
22 steps pass/3 fail, no missing definitions (/tmp/bdd-provider-red.log). Provider
Then shared feedback fails0 vs1; lifetime/header fail ungranted-send evidence.
Full per-Then member counterexample reconciliation requested before step6 mark.
Typed classification11 final run4 positive RED/7 compatible negatives pass; all7
negative false-positive mutation evidence above, /tmp/p2-typed-final-red.log.
Tool recovery: effort-change request for BDD agent timed out; get_state confirmed
agent alive and progressing, later effort medium confirmed. No duplicate worker
spawned and no completion inferred from tool timeout.
BDD follow-up extended every snapshot/transcript oracle member with counterexample
mutations and deadline None/Some helper controls. Final tagged rerun pending agent
report; no step completion claimed until observed.
Final BDD report retrieved:3 existing pass/3 new executable RED, all Then shared
oracle controls/counterexamples executed before scenario RED. No missing steps,
no replacement scheduler, no sleep absence oracle. Deadline production observations
remain masked with explicit None/Some sensitivity proof, not claimed GREEN.
Step6 evidence complete across new assertions; no temporary mutation residue.
Post-GREEN actual algorithm mutation matrix remains required.
GREEN integration found unused standalone fallback would not satisfy shared-state
requirement. Added NoHint typed receipt and2 single-assert contracts for sibling
shared escalation/dedup before implementation: expected2002/actual0 RED,
/tmp/p2-group-fallback-red.log. Invalid initial fixture pacing0 corrected1 before
behavioral RED. Policy agent implementing group-owned state, no leaf counter.
Shared success-reset integration test added once group code arrived: targeted
baselineGREEN (not claimed RED); policy owner requested success-reset-omission
mutation for2002→3002 before acceptance. Parent verified receipt34 GREEN,
/tmp/p2-receipt-parent-green.log.
