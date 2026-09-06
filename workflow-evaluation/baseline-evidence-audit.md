# Baseline evidence audit

**Disposition: usable for evidence-based panel review, with a localized feature RED-output limitation; independent verification still needs to be attached.** This is an archive/provenance audit, not a score, critical-gate decision, or template vote. The reviewer participated in evaluation design and must not serve as an independent scoring judge.

Scope: static inspection of all five `runs/*-base-001/` archives, frozen manifests/prompts/templates, and template source. No worker code was executed, no container/worker was contacted, and no evidence artifacts were modified. Only this report was written. Paths below are relative to `workflow-evaluation/`; `@N` denotes the `ordinal` in the named run's `transcript/messages.json` (also retained in raw pages).

## Completeness and provenance

All five runs contain task text, setup/provision records, pre-prompt container fixture hashes, before/after fixture trees, two raw transcript pages, assembled messages, archive summary, state, statistics, and final responses.

- Each page pair reports successful retrieval and terminates with `hasMoreBefore:false`. Its sorted message union exactly equals `messages.json`; ordinals are contiguous from 1. All recorded tool-call IDs have corresponding results, with no orphan results. Statistics agree with message/tool counts.
- All five initial task messages exactly match both run `task.txt` and their frozen spawn-request prompt. The only subsequent user-role message is the completion handoff nudge described below.
- Every file listed by `freeze-manifest.json` still matches its SHA-256. Current template source also matches its recorded source hash in `design-manifest.json`.
- Each `fixture-before` hashes identically to both `setup.json.files` and `container-fixture-hashes.json`. All saved write payloads and edit replacements match the corresponding after files. No symlinks were found. Existing smoke tests and operator drafts remain byte-identical; investigate before/after trees are identical. Bytecode caches occur in feature/refactor and are predefined exclusions.
- Archived state reports the specified low effort/model and correct active template for every run. Provision records agree on Python 3.13.5 and image ID `b1f87e8917502e1963979a0ed43fd7866427c961c26aac0d1576a17774b63662`.

| Run suffix `-base-001` | Messages / tool calls | Selection call/result | Final completion result | Non-cache artifact differences |
|---|---:|---|---|---|
| investigate | 21 / 8 | @3–4 | @18, 4/4 | None |
| chore | 27 / 11 | @5–6 | @24, 5/5 | README.md |
| bugfix | 35 / 16 | @5–6 | @32, 5/5 | intervals.py; new tests/test_merge.py |
| feature | 38 / 17 | @3–4 | @35, 6/6 | search.py; new tests/test_limits.py |
| refactor | 37 / 16 | @5–6 | @34, 5/5 | receipts.py; new tests/test_receipts.py |

## Actual template guidance

All five saved `templates/builtin-*.json` definitions match the source tuples in `quecto-agentic-harness/src/domain/workflow/engine/templates.rs`, including step keys, labels, phases, guidance, and template metadata. Every step's observed label, displayed phase, and guidance matches its saved definition in order. Guidance-result ordinals:

- investigate: **4, 8, 14, 16**
- chore: **6, 8, 16, 20, 22**
- bugfix: **6, 20, 22, 26, 30**
- feature: **4, 6, 17, 27, 31, 33**
- refactor: **6, 16, 24, 28, 32**

No template reselection, skipped steps, or backtracking appears. This establishes observed instruction equivalence, not binary build provenance or a complete runtime configuration dump. Step completion establishes progression only, not substantive performance.

Each run also has a user-role completion nudge after its first final response (investigate @20; chore @26; bugfix @34; feature @37; refactor @36), followed by a bounded final handoff. Its wording requests summary/checks/artifacts/blockers/next action. This is consistent with enabled completion nudges documented in `LAUNCH.md`, not text inside the saved template. Preserve it and hold this harness behavior comparable in revised runs; the transcript alone does not carry authenticated message-origin metadata distinguishing a harness nudge from an operator message.

## Material evidence limitations and interpretation cautions

1. **Feature has real omitted tool-output content despite `warnings: []`.** At @18–21 two commands redirect output to `/tmp/search-red.txt`: first exit 127 (30 bytes), then exit 1 (6,539 bytes/104 lines). The second overwrites the same path. At @22–23 only the first 18 lines of the latter are read; the transcript explicitly says 86 more lines exist. Neither full temporary output is in the archived fixture tree. The visible excerpt does establish two target-related `unexpected keyword argument 'limit'` errors before implementation (@24), but does not establish the detailed causes of every error or the full RED summary. Do not infer the absent lines or treat the first exit-127 attempt as feature RED. Independent replay can corroborate behavior, not recreate evidence of the original output. Attach an already-preserved supplementary log if available; otherwise retain this narrow limitation explicitly. Archive-summary warnings inspect flags, not textual spill/read-limit markers (`archive_session.py`).

2. **No other material transcript truncation was found.** Bugfix's failing and passing test outputs are present (@18, @28); refactor's before/after test outputs are present (@22, @30). Feature's final test output is present (@29). Successful bash results lack an explicit numeric zero in their text but have `isError:false`; failing commands include exit codes.

3. **Distinguish failed command chains from lost evidence.** Feature @29 and refactor @30 show tests passing followed by Git failure (exit 129, non-repository). Refactor's later `git diff` and `git status` operands in that `&&` chain therefore did not run. Chore @10 fails at `git status` before source reads, then recovers at @12. These are observable execution facts, not archive defects; do not infer successful diff checks from checked workflow steps or final assertions.

4. **Absent worker checks are not missing transcript pages.** Chore records source/help comparison but no execution of the documented export example (@9–24). Investigate records the primary print probe (@11–12), but no separate executed override-removal probe before checking the challenge step (@15). Judges may consider available source reasoning; they must not substitute later evaluator execution for worker execution. This audit makes no quality determination about those choices.

5. **Practical provenance limits remain disclosed, not new blockers.** No separately hashed post-copy container inventory or `after-files.json` was present at inspection; after-tree integrity is supported by archived bytes and exact recorded edits, not independent transfer attestation. The committed history is not a full initial system/developer/tool-schema capture or live event stream. It lacks per-message timestamps and authenticated termination/launch events. `baseline-launches.json` remains a launch-time record marked `prompted`; idle state plus final responses establish archived completion, not timing. Shared agent-home/network exposure and absent continuous filesystem audit remain the limitations in `LAUNCH.md`. No evidence of archive corruption was found.

## Panel handoff

Provide the frozen rubric/task contract/template, raw pages plus ordinal-indexed messages, both fixture trees, and the forthcoming isolated verification outputs. Mark feature's partial RED output and the completion nudge explicitly. Retain both final responses per run. Independent verification should corroborate final artifacts without silently replacing original worker evidence. A separate pre-rendered diff or ideal infrastructure telemetry is not required to use these small, reviewable archives.

**No scores, qualification judgments, or template-change votes are issued here.**
