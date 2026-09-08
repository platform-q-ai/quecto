# P2 PR review cycles

PR1686 head289297ad; cycle1 finders0118 semantics/2f4 compatibility/04fb policy
fetched diff independently. Policy clean; dedup2 material findings, independently
refuted by verifiersb3ca redirect/0aea malformed terminal, both confirmedP2.
Exactly one submitted nonpending GitHub review5141806294 COMMENTED with two inline
findings (not approval): automatic307/308 replay bypasses per-send pacing;
Anthropic invalidJSON terminal event observer diverges parser.

Step16 fixes underway RED-first. No authoritative CI label yet. No merge allowed.
Issue1679 already closed upstream; PR explicitly does not claim P3/P4 completion or
change issue state. Initial commit/push hooks passed; no bypass.
Malformed-terminal5 tests:2actual assembledRED/2incremental compatible+oracle pass;
observer now event dispatch fallback JSON null matches Anthropic parser.5+existing15
GREEN (/tmp/p2-anthropic-malformed-{red,green}.log). Test assertions shared oracle
counterexamples cover success/error/receipts/finish state; no malformeddata replay.
Redirect fix chooses explicit SingleAttemptClient proof rather than inspecting
opaque reqwest debug or silently replacing injected settings. Builder retains
caller configuration, overrides redirectnone/retrynever, cloned pool retained in
runtime context. Binding API gains safe client; all current callers migrate
explicitly. Existing default factory/client remains unchanged. Automatic redirects
now rejected not silently followed; deliberate manual resend needs future admission.
Redirect24 independent enabled cases each RED2raw vs1 before forcing policies,
disabled24-loop control passed. Now25testsGREEN, /tmp/pr1686-redirect-matrix-{red,green}.log.
Safe client retained in runtime context through cohesive gate+client binding,
explicit supplied builder only. Additional preserving-settings/sensitivity evidence
and regression checks pending owner completion.
Parent safe-client migration runtime17/OAuth2/malformed5 and architecture46/
contracts242GREEN, no disabled callback loss. /tmp/p2-pr-runtime-safe-green.log,
/tmp/p2-pr-safe-contracts.log. Inward AttemptAdmission unchanged; HTTP proof is
infrastructure wrapper, configured builder retained.
Safe-client also overrides caller configured retry-every-response: mutation removing
retrynever yields3raw vs1, restoredGREEN; default header retained. Finalredirect26
GREEN. Parent full lib first4035pass/1unrelated cleanup temp-path NotFound; targeted
cleanup then full rerun4036GREEN, no production change to unrelated flaky test.
BDD6/30GREEN. Logs /tmp/pr1686-retry-red.log /tmp/pr1686-redirect-final-green.log,
/tmp/p2-prfix-lib{,-rerun}.log,/tmp/p2-prfix-cleanup-rerun.log,/tmp/p2-prfix-bdd.log.
Parent all affected leaf targetsGREEN after safeclient migration (error26/attempt65/
feedback16/fallback5/initiation4/OpenAI6/root5/retry2/observer15),
/tmp/p2-prfix-leaf-regressions.log. Runtime safe client is retained outside proposal;
candidate reload cannot substitute unproven transport or rebuild default client.
Redirect9 shared oracle controls/counterexamples prove sends/endpoints/header/
grants/status/finish/replayedbody/failure/incrementalterminal sensitivity; same
helpers real wire. Final35GREEN /tmp/pr1686-redirect-oracles-green.log. All step16
valid findings implemented with evidence, ready fast-gate fix commit/push.
