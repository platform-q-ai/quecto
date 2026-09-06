# Feature round-3 intake proposal/vote — final locked

**Select F1: ONE exact replacement of `feature.steps[key=intake].guidance`. Existing test_design stays unchanged.**

| Alternative | YES | NO | ABSTAIN | Disposition |
|---|---:|---:|---:|---|
| **F1** |**3**|**0**|**0**|**Selected**|
| F2 |3|0|0|Overlapping; not selected|
| F3 |3|0|0|Overlapping; not selected|

Rankings: J1 **F1 > F3 > F2 > STATUS_QUO**; J2 **F3 > F1 > F2 > STATUS_QUO**; J3 **F1 > F3 > F2 > STATUS_QUO**. F1 wins **2–1 vs F3**, **3–0 vs F2**; no tie-break needed. No goal conflicts reported or found. J2's minority preference for F3's concise baseline/preservation wording is preserved. All alternatives remain exact, no silent merge or synthesized replacement.

## Exact text

Key `intake`, field `guidance`.

**Old:**

> Restate the desired behavior, constraints, and how completion will be verified.

**New:**

> Before implementation, restate the desired behavior and constraints, and identify a focused pre-change check with its expected result that distinguishes the requested behavior from current behavior. If executable verification is unsuitable or unsafe, identify a discriminating source, data, or artifact comparison instead. Record this plan and complete intake, then obtain the pre-change evidence in Design verification before implementing.

## Evidence and mechanism

Within `../batch-29fe81/bundles/`: `run-74cd12/transcript/messages.json` **11–18,20–25** and `run-a3906f/transcript/messages.json` **13–18,20–27** show implementation before first retained workflow engagement, then explicit test_design guidance encountered retrospectively. Both original templates already required substantive RED in test_design; the observed omission is not merely absence of that instruction.

F1 makes intake explicitly prospective: record a focused, discriminating plan and expected result, complete intake, then obtain pre-change evidence in the existing verification step. It does not require completing a later hidden step before intake, rerunning the same check twice, or producing artificial failure. Source/data/artifact alternatives support appropriate non-executable or unsafe verification cases. No task-prompt coaching or harness/guard change.

Transfer example retained by proposer: for a configuration-only retry-policy change, identify the current configuration-to-schema mismatch and expected compliant state at intake, then substantiate that comparison in Design verification before editing. For executable features, the existing test_design instruction still requires a meaningful target-caused failing check where applicable.

## Risks and limitations

A recorded plan can be superficial; additional wording/repetition can reduce salience; comparison exceptions must not displace practical executable verification without justification. Most importantly, no text can guarantee that initially bound guidance is read before editing. Binding-versus-selection, guidance delivery/activation salience, and reminder mechanics remain explicit confounds. This is an **unvalidated hypothesis**, not proof of causal improvement. Future evidence must show prospective intake and target-sensitive pre-change verification, not only higher scores or late honest disclosure.

Cross-panel B1/B2 anchor variability remains preserved. No rescore toward threshold, rubric change, or retrospective evidence relabeling. Independent same-model proposals and votes, not provider diversity; authorship withheld but self-recognition remains possible.

## Artifacts / preservation

- `selected-proposal.json`: exact keyed text, vote and unchanged-test_design disposition.
- `vote-aggregate.json`: all votes, rankings, pairwise decisions and conflicts.
- `alternatives.json`: exact submitted alternatives with evidence, risks and transfer examples.
- `proposal-attribution-private.json`: provenance, withheld during voting.
- `judge-{1,2,3}/{proposals,votes}/`: independent sealed outputs and full raw sessions. Bare reports read before archival. Latest vote sessions **55/51/58 messages**, **3 pages each**; complete final outputs recovered from truncated report displays; no collapsed/truncated raw content.
- `final-audit.json`: goal/integrity review.

**No source/candidate/rubric/score changes. Parent owns implementation.** Concurrent Bugfix base003 score separately locked **100/PASS**, report `../bugfix-followup/SUMMARY.md`; no Bugfix proposals initiated.
