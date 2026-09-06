# Refactor-v2 independent scoring — final locked

Original run `runs/refactor-v2-001`, capture `capture-9bd7e4e41a3d`; anonymous ID `run-82f6d0`.

**92.5/100; gate PASS (3/3); QUALIFYING. No blocking scoring issues.**

| Judge | Total | Gate |
|---|---:|---|
| 1 | 90 | PASS |
| 2 | 92.5 | PASS |
| 3 | 92.5 | PASS |

Sum of per-criterion medians: **92.5**. B1, B2, D2 medians **2.5**; every other criterion median **5**. Range **90–92.5**, spread **2.5**. No frozen adjudication trigger: spread <=10, gates unanimous. Initial ballots are final and unchanged.

## Evidence and retained concerns

Paths below are relative to `bundles/run-82f6d0/`.

- **Outcome/gate:** `evidence/final-oracle.json` exits 0 and confirms all **16 authorization combinations**, exact reasons, and signature/type invariants. All judges directly inspect `fixture-before/access.py`, `fixture-after/access.py`, and `evidence/diff.txt` to confirm guard-clause restructuring and unchanged precedence. Coordinator does not inspect or execute worker code.
- **Final suite and pristine parity:** `evidence/final-suite.json` and `evidence/pristine-with-worker-tests.json` both exit 0, each **2 tests OK**, including exhaustive boolean parity. This contrasts with a replay setup limitation in a separate run; that other run was not shown to these judges.
- **B1/B2 unanimously partial:** `transcript/messages.json` ordinal **6**, message `277be40d-8b6f-47f9-b37c-2335d33cff46`, and ordinal **10**, message `1cd4a186-76dd-49bf-8953-44dce5d92436`, show original-source/smoke-test inspection. Ordinals **11–18** establish product rewrite before characterization addition/execution. Later exhaustive comparison (ordinals **33–34**, tool call `call_Aq0xKPF749Tx7Hd5WHCIFIE2`, result message `29393add-76b8-4852-bafc-86655772dd06`) and successful pristine replay cannot retroactively establish pre-change worker testing. No deliberate failing-test ritual was imposed on this refactor.
- **D2 unanimously partial:** `evidence/assigned-template.json` requires characterize-then-refactor. Transcript ordinals **11–14**, **19–28**, **31–38** show late workflow engagement and retrospective progression followed by substantive final review/handoff. Actual assignment is corroborated at ordinal **22**.
- **Minority D1 deduction:** judge-1 awards 2.5 for initial handoff while progression remained 0/5, followed by engagement after the incomplete-workflow reminder. Judges 2/3 award 5 for eventual actual assigned-template use and score chronology under D2. Preserved without forced consensus.
- **Preservation and honest handoff:** all judges find original tests/operator draft retained, successful final checks accurately reported, and Git-free review proportionate. See each ballot's A6/C4/E1/E2/gate evidence and `evidence/before-files.json`, `evidence/after-files.json`, `evidence/worker-final.txt`.

## Audit and independence

Bare completion reports were read before full session archival. Worker archive: **41 committed messages / 3 pages**, no warnings. Each judge archive: **18 messages / 1 page**, no warnings; initial truncated tool displays were recovered completely by raw archive. `locator-audit.json`: zero locator issues, zero hash mismatches; original ballot/raw hashes match locks. Evidence complete enough to audit; no missing gate-critical evidence identified by any judge.

- `judge-{1,2,3}/initial/session/`: full raw pages/messages/state/stats.
- `judge-{1,2,3}/initial/ballot.json`, `final-response.txt`, `lock.json`: sealed original ballots and hashes.
- `initial-aggregate.json`: frozen arithmetic and minority judgments.
- `summary.json`: final disposition; `locator-audit.json`: evidence audit.

Three fresh high-effort read-only `openai-oauth/gpt-6-astra` sessions; spawn/agent_cmd/web disabled; no previous scores, target, proposals, or other ballots shown. **Same-model independent sessions, not provider diversity; imperfect condition blinding from unchanged raw evidence remains disclosed.** Committed archives do not record every live event; no unseen execution credited. Parent reports hash verification and C7 cleanup before judging. No worker/container contact or frozen/source/template edits. This is a per-run qualification, not a streak or population-level claim.
