# Feature round-2 proposal/vote result — final locked

**Selected F1, one exact replacement of `feature.steps[key=test_design].guidance`.**

| Alternative | YES | NO | ABSTAIN | Disposition |
|---|---:|---:|---:|---|
| F1 |3|0|0|Selected |
| F2 |3|0|0|Eligible but not selected: overlaps F1 |
| F3 |3|0|0|Eligible but not selected: overlaps F1 |

Every judge independently ranked **F1 > F3 > F2 > STATUS_QUO**. F1 wins each pairwise comparison 3–0; no tie-break necessary. No goal conflicts reported or found in coordinator review. Three separately submitted alternatives preserved unchanged; no synthesized or silently merged text. Author attribution was withheld during voting; self-recognition cannot be excluded. Independent proposals preceded exposure to others; independent votes preceded cross-vote disclosure.

## Exact selected text

Step key: `test_design`; field: `guidance`.

**Old:**

> Decide which tests, examples, or checks will prove the behavior before implementation.

**New:**

> Before implementation, choose and run a focused check that fails because the requested behavior is absent or wrong, not because setup is broken. Record the expected behavior and observed result before completing this step. Where executable RED is unsuitable, record a discriminating source, data, or artifact comparison instead. If implementation already occurred, disclose the missed ordering; later checks are not pre-change evidence.

## Evidence, transfer, risk

Feature bundle at `../batch-4f8c2e/bundles/run-63ab9e/`: `transcript/chronological.txt` ordinals **11–18** show implementation before tests, unrelated interpreter setup failure rather than target RED, then passing checks. Ordinals **24–27** show retrospective verification-design completion; **33–34** show genuine final verification. `evidence/pristine-with-worker-tests.json` establishes only post-run target sensitivity. Exact candidate-reference old guidance already asked for before-implementation planning, but not execution of a failing check.

F1 adds focused target-caused pre-change evidence, expected/observed result, suitable non-executable alternatives, and honest late-entry disclosure. It is an **unvalidated hypothesis**, not proof of improvement. Example transfer: before adding an optional export mode, observe an intended-API missing/wrong-behavior check; for a documentation/configuration-only change, use a discriminating authoritative source/artifact comparison rather than manufacture RED. Full proposer transfer examples/costs are retained with each alternative.

Risks: more verification effort/text; unrelated setup failure or nondiscriminating alternatives may still be misclassified; guidance encountered only after implementation cannot produce historical pre-change evidence. **Binding-versus-selection, initial guidance salience, and reminder mechanics are explicit confounds**; neither failure nor future benefit can be attributed solely to wording. Future fresh runs must establish target-sensitive evidence before first implementation edit; later passing suites alone do not validate the hypothesis.

## Artifacts / integrity

- `selected-proposal.json`: exact keyed old/new recommendation and votes.
- `vote-aggregate.json`: independent votes, conflicts, rankings and pairwise selection.
- `alternatives.json`: three exact submitted alternatives and full supporting rationale; `proposal-attribution-private.json`: preserved provenance.
- `judge-{1,2,3}/proposals/` and `/votes/`: sealed outputs, locks, full raw sessions and supplemental recovery. Bare reports read before archival. All collapsed tool content recovered (latest vote sessions 50/30/0 messages); raw originals unchanged.
- `final-audit.json`: goal/integrity check and archive recovery.

Same-model independent sessions, no provider diversity; proposal/vote self-recognition and imperfect original condition blinding remain limitations. No rescoring, source/candidate/rubric changes, worker/container contact, or benchmark prompt edits. **Parent owns implementation and subsequent validation.**
