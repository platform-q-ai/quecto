# P2 precommit verification / AC evidence

Base c3b597e0. No live provider calls; loopback clients unproxied. Mutation details
and initial RED per assertion: red-evidence, receipt-red, attempt-red, feedback-red,
local-review, runtime-integration and leaf-green-progress notes.

|AC / semantic inventory|Objective verification|
|---|---|
|AC3 P2-01/06 per leaf send|attempts65 exact preceding grants/raw sends all surfaces; retry2 exact3+2 sends sleeps outside permit; OAuth2 actual runtime rebuild preserves gate|
|AC3 P2-11 identity/reload|runtime17 explicit endpoint/account aliases shared/distinct, unknown reject, active+cooldown retained, changed C/map/interval/fallback-base/disable rejected, no-op retains config/context|
|AC4 P2-02–05 ownership|attempts65 receiver-return/drop, queued cancel, grant race, deadlines, blocked headers/body/fullchannel, >64 ordered deltas; actual OwnedTransport teardown trace mutant reversed fails|
|AC4 P2-06/12 no retry/tool wrapper|retry2 and actual OpenAI initiation4 retain existing owners; full lib4035 includes agent-loop/tools/delegation regressions; inward gate called only leaf HTTP, not tool/wait paths|
|AC5 P2-07/10 receipt/group|feedback16 HTTP header-before-body, independent group; receipt34 contracts max/idempotence/unavailable/clock; group fallback6 configuredbase/shared sibling/reset; fallback11 arithmetic jitter boundaries|
|AC5 P2-08 typed events|OpenAI6, rootResponses5, actual initiation4 billing/opaque retry/partial no replay; observer15 provider-specific dispatch/ignored extension/terminal; HTTP fallback5 opaque429/529 and live assembled advice|
|AC5 P2-09 hints|parser64 numeric/ms/three date forms/invalid/zero/past/overflow/excessive/max/repeated/case/capturedwall; typed11 classification contracts plus root precedence tests|
|P2-13 compatibility|differential26 enabled/disabled vendor/surface error strings/classifiers/status4KiB/read failure/EOF/final line; characterization4/transport2; provider481 regressions|
|BDD|@inference-admission6 scenarios30steps GREEN, actual loopback + shared Then sensitivity controls|
|Architecture/quality|architecture46/contracts242; strict workspace clippy alltargets+test-support -Dwarnings +complexity/argument/line warnings; fmt/checkquality/tag checks GREEN|

Step10 final commands logs:
- /tmp/p2-precommit-tests.log:15 targeted integration targets allGREEN
- /tmp/p2-precommit-clippy.log: strict workspace clippy GREEN
- /tmp/p2-precommit-bdd.log:6/30GREEN
- /tmp/p2-precommit-quality.log and /tmp/p2-precommit-tags.log:pass
- full harness lib4035 prior restored run /tmp/p2-local-final-lib.log

Local reviews: three independent low-effort angles and affected-angle reruns
recorded in local-review. All material findings addressed with RED/mutation and
restoredGREEN; final architecture, semantic and removed-behavior reviews clean.

No production activation or host/broker correctness claim. P3 IPC/durability/
container ownership and P4 rollout remain separate. RFC850 dependency fixed-year
mapping explicitly documented in ADR0026. No merge authorized.
Step11: only changed harness crate PATCH0.107.2→0.107.3, lock synced; no README
version line changes, no hardcoded crate version assertions found. cargo check
harnessGREEN (/tmp/p2-version-check.log). Other crate versions unchanged.
Step12 hook blocked new runtime test759lines. Extracted coherent counterexample
unit module to common/admission_runtime_oracle_tests.rs, main653lines; unchanged17
runtime assertionsGREEN (/tmp/p2-runtime-extract-green.log). No gate bypass; commit
retried after staging exact extraction and note.
