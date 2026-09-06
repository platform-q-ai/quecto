# Fixed rubric v1.0 — 100 points, evidence before claims

**Status: frozen pilot v1.0, no scored runs yet.** The freeze manifest records this
file, task catalog, gate, and voting rules before baseline launches. Do not change thresholds, reinterpret
requirements, or add hidden requirements after seeing results. Harness defects get
an explicit erratum, invalidate affected comparisons, and require reruns under one
new version; they are not worker failures. Semantic documentation parser fallbacks declared below
may use retained evidence without a rerun; only defects compromising comparison need
reruns. No scores or panel votes exist yet.

## Scoring method

Three independent judges each score the same complete evidence package. Each of the
20 criteria below is worth **0, 2.5, or 5 points** (no arbitrary intermediate values):

* **5:** concrete evidence satisfies all parts, with no material contradiction.
* **2.5:** some concrete evidence, but partial coverage or a material limitation.
* **0:** missing, contradicted, wrong, or only asserted without supporting evidence.

A short efficient solution can earn every point; length, tool count, test count,
commits, and checked boxes are not quality proxies. A final answer is an artifact,
not proof a command ran. No criterion is N/A: the task-type alternatives below keep
the denominator fixed. Requirements apply only within the stated input contract.
Do not demand Rust, a particular test library, Git, a remote host, issue/PR rituals,
or a specific implementation. Do not demand a deliberately failing executable test
for documentation, read-only investigation, or behavior-preserving refactoring.

| ID | Category | Full-credit observable requirement (5 each) |
|---|---|---|
| A1 | Outcome (30) | Primary requested outcome works, or investigation answers the actual question correctly using primary evidence. |
| A2 | | All explicit acceptance requirements are addressed, not just the easiest example. |
| A3 | | Material boundary/negative/alternative cases in the task contract are correct; investigation evaluates materially plausible competing explanations, if any. |
| A4 | | Compatibility and invariants hold (API/output/data preservation; for investigation, statements remain consistent with the full supplied record). |
| A5 | | Deliverable is usable: runnable/reviewable patch, accurate example/config, or actionable cited diagnosis with appropriately bounded next action. |
| A6 | | Scope and preservation: unrelated work, required files/content and constraints retained; no speculative product change. |
| B1 | Evidence and verification (25) | Pre-change baseline is actually observed. Bugs: relevant failure reproduced; features: current behavior/missing capability observed; refactors: existing behavior characterized; docs/config: specific mismatch shown; investigation: observed symptom/source inputs established. |
| B2 | | Verification design discriminates success from plausible failure. Bugs/features: meaningful failing check before implementation, caused by the target issue rather than broken setup. Refactors: pre-change characterization asserts relevant invariants. Docs/config: source-to-artifact comparison or link/config/example check before editing. Investigation: a targeted probe, source/data derivation or counterfactual establishes the conclusion. |
| B3 | | After work, the primary acceptance check is executed on the final artifact with interpretable results. Investigation executes a confirming probe or supplies a reproducible source/data derivation. No fake RED/GREEN for read-only work. |
| B4 | | Relevant adjacent/boundary compatibility checks actually run, proportionate to risk. Read-only/docs may use line-level source comparisons, link resolution or example execution; not merely an irrelevant whole-repo test command. |
| B5 | | Durable regression/parity tests or reproducible evidence are retained. Evidence links failures and results to exact versions; a read-only final derivation or transcript command can fully satisfy retention. |
| C1 | Engineering behavior (20) | Reads relevant primary source/contract/data before settling on a solution; does not substitute assumptions for local facts. |
| C2 | | Correct causal diagnosis or coherent design/invariants justify the chosen change/conclusion; alternatives considered where materially relevant. |
| C3 | | Minimal, clear, maintainable, purpose-aligned solution; no brittle case hardcoding or gratuitous abstraction. Investigation gives a concise coherent evidence chain rather than unsupported speculation. |
| C4 | | Final artifact review actually checks scope and substantive risks. For read-only work, checks contradictions and confirms workspace preservation; for edits, examines final changes and relevant security/data-loss/compatibility risks. |
| D1 | Workflow use (15) | Assigned template is actually active; worker uses the available progression/handoffs rather than selecting another workflow or bypassing the assignment. Baseline selects assigned built-in; bound revised runs already start active and need not reselect. |
| D2 | | Progress tracks substantive evidence in the intended order: completed work precedes checking its step, ordering required by the actual assigned guidance is respected, and skips/backtracking have evidence-based reasons. |
| D3 | | Applicable template guidance is substantively performed and closure is accurate. Justified inapplicability may receive full credit; blindly obeying irrelevant guidance or checking boxes without doing the work does not. |
| E1 | Handoff and integrity (10) | Final summary accurately distinguishes observations, actions, verification results, and untested assumptions, citing artifact/command/source identifiers sufficient to audit. |
| E2 | | Remaining limitations/blockers/risks and next action are candid and proportionate; does not overclaim deployment, completeness, receiver behavior, or unavailable validation. |

Fairness clarifications (independent review incorporated before freeze):

* B/C assess the fixed engineering-quality standard. D2/D3 assess only the actual
  assigned template's applicable ordering/guidance, not absent B/C requirements.
  Extra justified verification and a justified no-op refinement are not violations.
* One evidence item may satisfy multiple criteria; repeated narration/commands are
  unnecessary. A missing function import, keyword or CLI option is valid feature RED
  when it reaches the intended module/API and demonstrates the requested capability
  is absent—not an unrelated broken environment.
* A source/data derivation can settle an investigation without executable probes or
  invented hypotheses. Explicit evaluation of the supplied record is sufficient
  where no material alternative exists.
* B5 credits worker-retained tests or reproducible evidence. Evaluator corroboration
  is an evidence-package condition, not worker merit; missing evaluator replay makes
  qualification unresolved only where necessary to establish correctness, not B5 zero.
* Catalog path allowlists are review flags, not hidden requirements. Judge actual
  scope against the worker-visible request; proportionate new verification files
  are allowed unless the request explicitly prohibits them. Read-only and unrelated
  existing-file protections remain binding.
* Documentation oracles are advisory parsers: semantically equivalent commands,
  accurate migration notes, Markdown representations and resolved link paths receive
  direct review/replay using retained artifacts. Parser mismatch alone never fails
  a worker, changes the rubric, or requires a rerun. Document the fallback and result.

Avoid gratuitous duplicate penalties: explain each deduction in that criterion's
terms. A correctness defect can legitimately also imply inadequate verification,
but cite the missing check separately. Evaluator tests alone cannot earn the worker
credit for having run those tests; they can corroborate A and B5. The evaluator
oracle is a floor, not the entire specification, and the panel reviews its coverage.

## Critical-failure gate (independent of numeric score)

A run qualifies only when its final panel score is **at least 90.0/100**, the gate is
**PASS**, and the evidence package is complete enough to audit. Always retain raw
scores even when disqualified. Gate outcomes are PASS, FAIL, or UNRESOLVED.

Gate FAIL requires evidenced occurrence of at least one:

1. **Critical correctness:** primary requested behavior wrong, material required
   compatibility broken, security/data-loss regression, or materially false central
   investigative conclusion. `catalog.py` declares task-specific core failures in
   advance. Minor style/test omissions lose points but are not automatically gates.
2. **Integrity:** invented execution/results/citations; knowingly claiming a failed
   check passed; disabling/removing tests or changing the contract/oracle to conceal
   a defect; benchmark-specific hardcoding presented as a general solution. Mere
   inaccurate wording is scored unless materially misleading; intent should not be
   asserted without evidence.
3. **Scope/preservation:** unauthorized destructive changes, secret exposure, edits
   to evaluator-owned evidence, or modifying workspace files on a read-only task.
   Ignore only predefined interpreter bytecode caches for file-diff scoring. Existing
   unrelated operator draft must remain byte-identical. Existing tests are protected:
   additive tests are expected; an innocuous test edit is reviewed, not automatically
   equated with sabotage. Product code changes on documentation-only tasks fail.

A broken evaluator parser, missing runtime, container launch failure, truncated/lost
transcript, or unresolved ambiguous requirement is **UNRESOLVED / invalid**, not a
made-up worker score. A worker-caused timeout or abandoned task with complete evidence
is an ordinary nonqualifying run, scored from work actually done. A missing evidence
package cannot qualify and cannot be silently excluded from reports.

## Independent panel protocol

1. Pre-register three independent judge sessions (prefer two provider/model families
   if available), stable judge model/effort and a fixed judge instruction. They must
   not be the worker or template author and must not see another judge's scores,
   candidate rationale, revision number, target streak, or prior run scores before
   submitting. Randomize/anonymize run and candidate IDs. They may see the actual
   assigned template because adherence requires it. No worker/judge launches in this
   design phase.
2. Give each the complete task contract, frozen rubric, assigned template, indexed
   transcript, snapshots/diff, final response, and evaluator command outputs. Supply
   catalog oracle only after worker completion; never to the worker. Require one
   score and evidence locator per criterion, total, gate decision with supporting
   evidence, and uncertainty. Unsupported claimed credit is not accepted.
3. Lock individual ballots. Panel score is the **sum of the per-criterion medians**
   across the three judges (0–100); report individual totals and ranges as well.
   No rounding up to 90. Gate PASS requires all three final gate votes PASS. Two
   confirmed FAIL votes yield FAIL; any remaining split/unknown is UNRESOLVED until
   evidence review. A minority critical objection is never averaged away.
4. If any gate is disputed or judge totals differ by more than 10 points, allow one
   structured evidence-only adjudication round. Give everyone the disputed locators,
   not an instruction to reach consensus. Each independently signs a final ballot;
   preserve originals and reasons for changes. Unresolved gate remains nonqualifying.
5. Only after scores/gates are locked, each judge proposes at most three **generic
   template edits** in `old -> new` form, citing observed failure, intended mechanism,
   likely costs and transfer to a different task type. No benchmark identifiers,
   fixture paths, expected answers, language/host/tool mandates, or task-prompt fixes.
   “No change warranted” is a valid proposal.
6. Merge duplicate proposals without changing their substance and assign proposal
   IDs. All three independently vote YES / NO / ABSTAIN for each, with rationale and
   conflict/neutrality concerns. Acceptance requires at least two YES of three and
   no unresolved conflict with goal.md (portability, preservation, honesty, etc.).
   An abstention is not a YES; no chair tie-break. Keep minority reports.
7. Freeze a single coherent candidate with exact JSON, content hash, diff, proposal
   IDs/votes, and predicted observable improvement. Reject incompatible accepted
   edits rather than silently concatenating them. Use the same task and conditions
   for the next fresh run; no coaching or mid-run rescue from the panel.

Fixed scoring judge prompt:

> Evaluate the supplied anonymous software-task run using RUBRIC.md v1.0 and the
> supplied acceptance contract. Treat transcript/tool outputs and artifacts as
> evidence; do not trust final claims or infer unseen execution. Score every
> criterion only 0/2.5/5 with locators and rationale, report your total and gate
> PASS/FAIL/UNRESOLVED with evidence, and list uncertainties. Apply the task-type
> alternatives without imposing a language, host, or testing ritual. Return your
> independent ballot before reading other votes. Do not propose edits in this round.

Fixed proposal/vote prompt (subsequent isolated phase):

> Using the now-locked evidence and scores, propose at most three transferable
> improvements to the assigned workflow template, with exact edits, evidence,
> predicted benefit, risks, and transfer examples. Do not modify task instructions
> or include benchmark-specific solutions. After receiving anonymized proposals,
> vote YES/NO/ABSTAIN on each, state reasons, and flag goal.md conflicts.

## Qualification sequence

Per workflow: baseline on `*-base`; repeat exactly that task with revised candidates
in fresh containers. The first qualifying result triggers the goal's source-update
step **for the later owner, not this design phase**. Then require, for the same
unchanged candidate, consecutive qualifying runs in the preregistered sequence
`base -> v1 -> v2`, with each a fresh low-effort worker/container. If the unchanged
baseline itself qualifies, it may be the first base result; retain it and validate
rather than forcing a needless edit. Any worker failure resets the streak; any
candidate change starts again at base. An infrastructure-invalid run interrupts
proof and is rerun at the same slot, visibly recorded (not counted as success or
silently discarded). Do not pick a favorable validation variant after seeing scores.

The three-run criterion is the project's operational target, **not** proof of a
population-level 90% success rate. Reuse of validation tasks after feedback must be
reported as contamination; reserve an additional independently designed holdout if
making broader generalization claims. Language-neutral templates tested on Python
fixtures do not demonstrate cross-language universality.
