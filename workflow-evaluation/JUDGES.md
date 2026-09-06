# Independent judge launch and bundle recipe

No judges spawned here. Use the three prompt files in `judge-prompts/` as independent
spawn tasks; replace BUNDLE_PATH and ANONYMOUS_RUN_ID with the same evidence bundle
and random identifier for a given run. They differ only by judge ID, not scoring
standard or assigned expertise. Launch three fresh sessions, not workers/template
authors, with identical model/effort and available evidence. Practical supported
choice: `model: openai-oauth/gpt-6-astra`, `effort: high`, `read_only: true`,
`workflow: false`, disable spawn/agent_cmd/web tools. One model family is acceptable
for this pilot; independence is separate sessions and sealed ballots, not fictional
provider diversity. Record that shared-model correlation limitation.

## Parent steps

1. Parent already read bare reports. Invoke, once per completed run:
   `python3 -B workflow-evaluation/capture_and_verify.py workflow-evaluation/runs/<run>`.
   Existing transcript and fixture-after stay untouched. Unique capture directory
   contains verification and a hashed `bundle-index.json`; inspect `status.json`.
   Exit 0 means capture completed, NOT checks passed or worker qualified. Exit 2
   means incomplete evidence/infrastructure; retain results and judge only supported
   claims. A retry creates a new directory and never overwrites older evidence.
2. Reopen transcript pages and fixture bytes using the hash index before cleanup.
   Review nonzero check exit codes, any timeout, and missing/truncated evidence.
   The helper does NOT clean containers or disposable directories: parent decides
   cleanup after capture verification. A subprocess timeout may leave work inside
   the container; no unsupported claim of terminating it is made.
3. Build one anonymous bundle per run OUTSIDE worker-visible directories with:
   - exact `task.txt` (including workflow selection assignment);
   - frozen rubric and `acceptance.json`, frozen oracle source;
   - actual assigned-template JSON if retained; otherwise expected built-in snapshot
     explicitly labeled EXPECTED and transcript guidance as corroboration;
   - `fixture-before/`, `fixture-after/`, `changes.json`, `diff.txt`;
   - `transcript/` original pages and messages, transcript index, worker-final text;
   - all final-oracle/final-suite/pristine-with-worker-tests command JSON records,
     copy/allocation error records, replay selection and capture status/limitations;
   - a sanitized run-context JSON: actual model/effort/image, relevant settings and
     known deviations; missing metrics explicitly null, not estimated;
   - `bundle-index.json`, with paths rebased and hashes recomputed if redacted.
   Do NOT include previous scores, revision numbers, streak, candidate rationale,
   preparation history, other task solutions or the other judges' ballots.
4. `bundle-index.json` emitted by capture is the audit inventory rooted at original
   run-dir, not automatically anonymous. Parent creates `INDEX.md` in the bundle:
   map each entry to its bundled path; list missing evidence and original hashes.
   Remove identifying run-directory/condition labels from displayed index/context;
   preserve immutable raw originals separately. Absolute paths in command records
   may reveal the original run; disclose residual imperfect blinding rather than
   fabricate a fully blinded experiment. Never alter substantive tool evidence.
5. Supply identical bundle bytes to each judge, with different output ballot location
   or final-response-only ballots. No judge reads other ballots. Preserve each
   original response before adjudication. Parent can use paginated archival for
   judge sessions as well. Use the frozen per-criterion medians/gate rules, not a
   newly invented aggregation. Proposal/vote phase follows only after ballots lock.

## Verification interpretation

Final checks execute in separate container copies of the captured artifact. The
pristine replay overlays only added/modified `tests/**` onto pristine product files;
original deleted tests remain. This can show regression sensitivity or refactor
parity, but cannot prove the worker ran RED before editing. An import error for the
new capability can be meaningful RED; missing unrelated helper files is a replay
limitation. Other test frameworks, root-level tests or shell snippets need manual
post-run replay where worthwhile. No `tests/` is not a defect for read-only/docs.
No host execution of any worker module/test occurs in the capture helper.

Unavailable `git diff` in a fixture without Git is NOT itself a correctness or
adherence failure. Inspect whether the worker substantively reviewed artifacts with
an appropriate fallback, and whether actual template guidance was portable. Do not
pre-score or infer that the review happened merely from a final claim. Parent has
reported Git unavailability in some final reports; this is an observation to check,
not a panel conclusion. Structural refactors still require actual code inspection.
