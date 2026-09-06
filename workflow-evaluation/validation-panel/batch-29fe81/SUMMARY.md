# Feature round-2 varied pair — final locked

| Run | Capture | J1 | J2 | J3 | Panel | Gate | Qualification |
|---|---|---:|---:|---:|---:|---|---|
| feature-v1-002 | capture-c8374b0ce484 |87.5|87.5|90|**87.5**|PASS 3/3|**Not qualifying**|
| feature-v2-002 | capture-ef2c781b4b53 |85|87.5|90|**87.5**|PASS 3/3|**Not qualifying**|

Frozen sum of per-criterion medians, not mean totals. Total spreads 2.5 and 5; no gate disagreement; **no adjudication trigger, no blocking scoring issue**. Initial ballots final and unchanged.

## Candidate streak (computed after lock)

Same exact assigned round-2 candidate hash across base003/v1/v2, verified in `candidate-continuity-private.json`.

**base-003 92.5/PASS → v1-002 87.5/PASS → v2-002 87.5/PASS.**

**Three-run criterion NOT MET; trailing qualifying count 0.** Full references/hash in `candidate-streak.json`. Gate correctness alone does not overcome the numeric requirement. No additional proposals automatically authorized.

## Criteria and exact evidence

Both runs have median **B1=2.5, B2=0, D2=2.5, D3=2.5**; all remaining 16 criteria medians **5**. All 20 per-criterion ballots, ranges, evidence and minority rationale preserved in `summary.json` and original `judge-*/initial/ballot.json`.

Locators below are relative to `bundles/`:

### v1 — run-a3906f
- `transcript/messages.json` **6,10,12–14**: source/smoke-test baseline inspection, then implementation; no executed missing-capability observation beforehand.
- **13–18**: implementation before new test execution; **25–27**: temporary reconstructed-original missing-symbol failure, candidly identified as retrospective. This is meaningful intended-API sensitivity, **not a setup error and not pre-change worker RED**. Temporary files deleted by worker were not recovered/recreated; retained command/output, original fixture and test supply auditable evidence.
- **33–40**, plus `evidence/assigned-template.json:14–41`: later review/verification/closure supported, but the applicable prospective check remains missing. Disclosure preserves honesty, not chronological completion.
- Complete Git-free before/after review supports C4. Final oracle and suite pass; evaluator pristine replay corroborates test sensitivity only.

### v2 — run-74cd12
- `transcript/messages.json` **6,10–12**: original text-only CLI/source read before edit; **11–18**: implementation precedes test creation/execution.
- **25**: worker expressly discloses missing pre-change RED; **31–36**: substantive later verification/closure. `evidence/replay-selection.json:2–5` and pristine replay do not supply chronology.
- Exact assigned guidance at `evidence/assigned-template.json:14–41` remains only partially substantively fulfilled. Correct final CLI behavior, compatibility, boundaries, and preservation verified by judges and retained evaluator checks.

### Minority and cross-panel interpretation

- Judge-3 assigns **B2=2.5 in both runs** for discriminating post-change tests while explicitly denying chronological RED. Judges 1/2 assign **0**, the frozen median.
- Judge-1 additionally assigns **C4=2.5 in v2**, finding final CLI source review but no whole-change review covering the rewritten existing test and unrelated-content preservation (`transcript/messages.json` **29,31–33**; `evidence/diff.txt:19–67`). Other judges award 5. Minority remains unchanged.
- Base003 independent panel awarded **B1/B2 medians 5/2.5**, this panel **2.5/0**. Both panels agree no worker pre-change RED. Task/session variation and anchor interpretation limit score comparisons; do not claim the candidate deteriorated or improved causally. No cross-run rescoring toward a threshold, and between-panel disagreement is not the frozen within-run adjudication trigger.
- Binding versus selection, initial guidance salience and reminder mechanics also confound causal attribution. Honest late-entry behavior is observable, but cannot repair missing history.

## Preservation and independence

Three fresh high-effort read-only `openai-oauth/gpt-6-astra` sessions, same randomized two-run order; spawn/agent_cmd/web disabled. No targets, prior scores, proposals, other ballots, or coordinator chronology interpretation supplied. **Same-model independence, not provider diversity; imperfect condition blinding disclosed.** Within-session batch context can correlate judgments.

Bare reports read before full raw archives. Judge archives: **41/3, 37/2, 44/3** messages/pages. Complete final ballots recovered despite truncated report displays; **no collapsed/truncated raw messages, archive warnings, hash mismatches or locator issues**. Worker archives **43/3 and 39/2**, complete. No worker/container contact or coordinator worker-code execution/inspection. No source/candidate/rubric edits.

- `summary.json`, `initial-aggregate.json`: final disposition/arithmetic/medians/minorities.
- `candidate-streak.json`, `candidate-continuity-private.json`: streak and exact candidate continuity.
- `judge-{1,2,3}/initial/`: complete raw sessions, sealed final JSON/text and locks.
- `locator-audit.json`, `provenance.json`: evidence integrity.
- Parent owns any next proposal, source revision, or validation authorization.
