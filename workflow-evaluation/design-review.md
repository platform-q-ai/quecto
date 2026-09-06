# Independent evaluation-design review

## Review conclusion

This is a usable practical pilot design after a few pre-freeze clarifications—not yet a basis for broad “world-class” claims. The rubric appropriately separates observed execution from final claims, protects critical correctness, allows task-specific verification, and requires independent ballots. Keep those features; no elaborate infrastructure expansion is necessary.

Reviewed `goal.md`, `workflow-evaluation/{RUBRIC.md,TASKS.md,catalog.py}`, and `quecto-agentic-harness/src/domain/workflow/engine/templates.rs`. No workers were run or scored and no template-change votes were cast. This report is the only file written by this reviewer, pursuant to the follow-up authorization.

Citations below use `RUBRIC.md`, `TASKS.md`, and `catalog.py` relative to `workflow-evaluation/`; `templates.rs` means `quecto-agentic-harness/src/domain/workflow/engine/templates.rs`. Line references describe the reviewed version, before subsequent edits.

## Minimal fixes before freeze

### 1. Separate engineering-quality deductions from actual template adherence

The quality standard intentionally exceeds some current guidance:

- Feature says “Decide which tests … will prove the behavior,” not execute a failing check.
- Chore moves directly from scoping to changing.
- Bugfix has no explicit final artifact-review step.

Those differences are legitimate experimental subjects, but judges could deduct both for missing quality behavior and for allegedly disobeying guidance that never required it.

**Citations:** `RUBRIC.md:34–45`; `templates.rs:51–67,89–117,133–149`.

**Minimal rubric addition:**

> Score B/C against the fixed engineering-quality standard. Score D2/D3 against the actual assigned template’s applicable ordering and guidance; do not import absent B/C requirements into adherence. Additional justified verification is not a workflow violation.

Also specify that the same evidence may satisfy several criteria—separate commands or repeated narration are unnecessary. Keep separate deductions only where distinct requirements genuinely failed (`RUBRIC.md:49–53`).

### 2. Clarify meaningful RED and sufficient read-only evidence

B2 excludes failures caused by “broken setup.” For a new API, however, a correctly targeted missing function import, unsupported keyword, or unsupported CLI option can be exactly the expected feature failure. Conversely, investigate A3/B2 could encourage invented alternative hypotheses even when supplied source and inputs settle the issue.

**Citations:** `RUBRIC.md:30,34–37`; `catalog.py:342–374,381–387,413`; `templates.rs:25–35`.

**Minimal rubric addition:**

> Missing capability errors count as RED when the test reaches the intended module/API and the error is the capability being added. A source/data derivation may distinguish investigative hypotheses without executing code. Do not invent a competing explanation where none is materially plausible; explicit evaluation of the supplied record is sufficient.

This preserves evidence-first behavior without rewarding artificial testing rituals.

### 3. Do not enforce evaluator-only path restrictions as hidden worker requirements

Workers receive task bodies and fixtures, not acceptance cards. Yet the cards define exact editable/new-path allowlists. For example, chore-v2 forbids editing other existing files in its prompt, while its allowlist also excludes any new verification file. Chore-v1 likewise has a narrower evaluator allowlist than its explicit instructions.

**Citations:** `TASKS.md:5–12,42–46`; `catalog.py:196–209,221–228`; `RUBRIC.md:21–23,72–77`.

**Minimal fix:** Treat undeclared-path changes as review flags, judged for actual scope violation—not automatic worker failures solely because an evaluator allowlist rejected them. Alternatively, expose genuine scope restrictions in the task contract before freeze. Preserve the explicit read-only and unrelated-file protections.

### 4. Remove known oracle representation traps now

Two documentation checks constrain representation beyond requested semantics:

- Chore-base requires a standalone command beginning exactly `python3 export.py`, selects the first matching example, and bans obsolete option strings anywhere—even an accurate migration note.
- Chore-v2 requires exact link spellings, rejecting equivalent valid targets such as `./guide/install.md`.

**Citations:** `catalog.py:166–185,229–238`. The existing parser caveat is good, but currently emphasizes command extraction (`TASKS.md:37–40`).

**Minimal fix:** Check resolved targets and actual documented-command behavior, or preregister semantic direct review/replay for all documentation assertions, not extraction alone. A parser mismatch should not require another worker run when retained artifacts already establish the answer; reserve reruns for defects that genuinely compromise the comparison.

Also correct the feature-v2 wording “integer key 'lines'” to “key 'lines' with an integer value” before freezing (`catalog.py:413`; `TASKS.md:495`).

### 5. Close a small but material compatibility-oracle gap

Feature-base promises preservation of existing case-insensitive matching, whose implementation uses `casefold()`. All oracle examples are ASCII, so replacing it with `lower()` would pass despite changing valid string behavior.

**Citations:** `catalog.py:343–375`; compatibility standard at `RUBRIC.md:31,53`.

**Minimal fix:** Add a Unicode case-folding example, such as matching `"Straße"` against `"STRASSE"`, in both unlimited and limited modes. No new framework or generated test system is needed.

A second cheap precision fix: chore-v1 dictionary equality accepts `debug: 0` as equal to `False`; assert the required JSON boolean type rather than relying solely on Python equality (`catalog.py:213–215`).

### 6. Keep evaluator replay separate from worker merit

B5 combines durable worker evidence with evaluator-owned replay. That risks reducing a worker’s quality score for an evaluator omission, despite the explicit infrastructure-invalid policy.

**Citations:** `RUBRIC.md:38,51–53,79–83`.

**Minimal rubric clarification:**

> Award B5 worker credit for retained regression/parity checks or reproducible evidence. Independent corroboration is an evidence-package qualification condition; evaluator failure makes qualification unresolved, not the worker’s retained evidence deficient.

For read-only/docs work, confirm that a reproducible final derivation or retained transcript command can earn full credit without creating workspace files.

## Coaching, task difficulty, and interpretation limits

No obvious prohibited process coaching is present: task bodies do not instruct RED/GREEN or rubric-box completion, and launch differences are explicitly controlled (`TASKS.md:5–12`). Explicit product requirements and requests for regression coverage are legitimate.

However, substantial answer-location and solution scaffolding limits what this experiment measures:

- Investigate-base supplies the exact reproduction command and tells the worker where the relevant launch environment resides (`catalog.py:30–38`).
- Currency bugfix names the causal mechanism; falsy-override bugfix specifies the replacement semantics (`catalog.py:283,312`).
- Refactor variants prescribe the extraction or guard-clause strategy, rather than testing whether workers choose an appropriate restructure (`catalog.py:504`; `TASKS.md:619–629`).
- Interval baseline is a one-comparison defect; chore-v2 is two link repairs (`catalog.py:245–258,221–238`).

These are valid small-task behavior probes, not worthless tests. They can expose missing reproduction, weak verification, careless preservation, or dishonest closure. They cannot strongly establish diagnosis under uncertainty, large-change maintainability, integration competence, or workflow causality.

**Action:** Proceed with these baselines, label resulting 90/100 judgments “qualifying on the frozen small-task suite,” and retain the existing generalization caveat (`RUBRIC.md:154–158`). If broader claims are needed later, use a preregistered less-scaffolded task with a genuine cross-file dependency or competing explanation. Do not harden tasks after seeing baseline outcomes and retain their old scores as comparable.

## Principles for later evidence-backed panel voting—not votes now

1. **Require an observed failure mechanism:** connect each proposed instruction to transcript evidence, not merely missing words in the template.
2. **Prefer discriminating evidence over ceremony:** meaningful RED where applicable; source comparisons, counterfactuals, and parity elsewhere.
3. **Keep adherence distinct from quality:** checking every step cannot rescue wrong work; useful extra verification should not be punished.
4. **Make effort proportionate:** accept concise reasoning and justified no-op refinement; avoid mandatory speculative alternatives or irrelevant broad suites.
5. **Preserve transferability:** no fixture names, expected answers, tool mandates, or benchmark-specific edge-case lists in templates.
6. **Weigh costs and retain “no change”:** extra guidance must plausibly improve behavior enough to justify attention and execution overhead.

These principles follow the goal’s evidence, neutrality, and preservation requirements (`goal.md:9–24`) and the existing post-score proposal protocol (`RUBRIC.md:108–119`). Actual template proposals and votes should wait for the parent’s baseline execution evidence.
