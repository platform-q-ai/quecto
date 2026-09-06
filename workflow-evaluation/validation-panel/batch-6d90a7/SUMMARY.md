# Six varied runs — final locked

**Investigate and Chore complete three consecutive fresh qualifying candidate runs; Bugfix does not.**

| Run | J1 | J2 | J3 | Panel (sum of medians) | Gate | Result |
|---|---:|---:|---:|---:|---|---|
|investigate-v1-001|97.5|95|97.5|**97.5**|PASS 3/3|QUALIFYING|
|investigate-v2-001|97.5|95|97.5|**97.5**|PASS 3/3|QUALIFYING|
|chore-v1-001|97.5|95|97.5|**97.5**|PASS 3/3|QUALIFYING|
|chore-v2-001|97.5|95|97.5|**97.5**|PASS 3/3|QUALIFYING|
|bugfix-v1-001|90|85|87.5|**87.5**|PASS 3/3|NONQUALIFYING_SCORE_BELOW_90|
|bugfix-v2-001|90|85|87.5|**87.5**|PASS 3/3|NONQUALIFYING_SCORE_BELOW_90|

No adjudication trigger: total spreads 2.5 for investigate/chore, 5 for bugfix; no gate disagreements. All initial ballots final and unchanged; no blocking scoring issues.

## Candidate streaks (only computed after lock)

| Candidate | base-002 | v1-001 | v2-001 | Trailing qualifying count | Three-run goal |
|---|---:|---:|---:|---:|---|
|investigate|97.5/PASS|97.5/PASS|97.5/PASS|3|MET|
|chore|95.0/PASS|97.5/PASS|97.5/PASS|3|MET|
|bugfix|92.5/PASS|87.5/PASS|87.5/PASS|0|NOT MET|

Each actual assigned-template byte hash matches its workflow's base-002 candidate; distinct v1/v2 tasks supply variation. Parent reports fresh execution and post-archive cleanup, retained execution context supports distinct runs. `candidate-continuity-private.json` and `candidate-streaks.json` preserve exact hashes/references. This does not assert a population success probability. Refactor's separate parent-confirmed three-run completion remains unchanged; feature round-2 base-003 separately qualifies once at 92.5/PASS.

## Criterion medians and evidence

For **all four investigate/chore runs**, only **D2=2.5**; the other 19 criterion medians are **5**. For **both bugfix runs**, **B1=0, B2=0, D2=2.5**; other 17 medians **5**. Full 20-criterion votes, medians/ranges, locators and minority rationales are retained in `summary.json` and each sealed ballot.

Locators below are relative to the indicated bundle under `bundles/`.

- **Investigate v1 — run-5db209:** D2 retrospective scope/initial diagnosis: `transcript/messages.json` **5–17, 19–24**; exact assigned guidance `evidence/assigned-template.json:8–29`. Diagnosis is appropriately bounded; record does not establish receiver persistence, deduplication, downstream effects, or exact timeout/send internals. Source/data derivation is valid investigation evidence; no code execution ritual imposed.
- **Investigate v2 — run-26a8fc:** D2 retrospective scope: transcript **5–17, 19–26**; assigned guidance **8–29**. Local exact-case behavior is established; upstream policy/business intent remains outside supplied evidence.
- **Chore v1 — run-e107bd:** D2 scope after edit/initial handoff: transcript **9–19, 21–34**. Successful recovered interpreter/loader checks address configuration task; no live service/deployment claimed or required.
- **Chore v2 — run-904c3e:** D2 scope after edit: transcript **7–15, 17–26**. Shell verification supports relative-link task; unavailable `python` alias does not prove absence of every interpreter, and renderer execution is not required for the simple evidenced link semantics.
- **Bugfix v1 — run-b3a751:** implementation before executed failure at transcript **6–18**, reconstructed-original demonstration only **23–24**; progression **25–38**. B1=0 unanimously; B2=0 majority, judge-1 B2=2.5 for later sensitivity retained. Final integer-cent behavior correct; final oracle/suite success and pristine target failures corroborate artifacts, not pre-change chronology.
- **Bugfix v2 — run-ec6204:** implementation/test creation before failure at transcript **6–16**, original-expression substitution only **21–22**; progression **23–34**. Same B1/B2 majority distinction. Final falsey-value behavior is correct; later original-expression failure does not create pre-change execution.

Every oracle exits **0**. Bugfix final suites pass, and pristine replays fail for the target defects. Neither passing final artifacts nor retrospective replay satisfies missing chronological worker verification.

### Minority and cross-panel limitations

Judge-2 additionally awards **D1=2.5 in all six runs** for initial bypass/late workflow engagement; judges 1/3 score actual assigned-template use at 5 and timing under D2. Judge-1 gives **B2=2.5 in both bugfix runs**, while judges 2/3 give 0 for absent pre-implementation failing checks. Those minority ballots remain fully preserved, not ignored or changed to force consensus.

Base-002 bugfix's independent panel awarded B1/B2 medians 2.5; this fresh panel awards 0/0. All acknowledge absent chronological pre-fix execution. Panel anchor variability and task/agent differences limit causal score comparisons; frozen rubric and original scores are unchanged. The between-panel difference is not the frozen within-run >10/gate-disagreement trigger and no cross-run rescoring is authorized.

## Audit and independence

Bare reports read before full raw archival. Judge archives **43/3, 47/3, 56/3** messages/pages; full final JSON recovered despite truncated report displays. No collapsed/truncated raw messages, archive warnings, hash mismatches, or locator issues. Worker archives retained completely. `locator-audit.json` verifies preservation; `judge-{1,2,3}/initial/` retains final response, parsed ballot, raw pages/messages, state/stats and hash locks.

Three fresh high-effort read-only `openai-oauth/gpt-6-astra` sessions; six runs each, identical randomized run order, independent ballots. No prior scores, target, proposals, rationale, or other ballots supplied. **Same-model independence, not provider diversity; imperfect condition blinding and within-session batch correlation remain disclosed.** Binding-versus-selection/activation and reminder mechanics limit causal attribution. No worker/container contact, worker-code execution by coordinator, source/candidate/rubric edits, or automatic proposals.

Artifacts: `summary.json`, `initial-aggregate.json`, `candidate-streaks.json`, `candidate-continuity-private.json`, `locator-audit.json`, `provenance.json`, and complete judge sessions. Parent must authorize any new proposal/vote or validation work.
