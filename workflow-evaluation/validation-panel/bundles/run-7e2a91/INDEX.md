# Evidence bundle: run-7e2a91

Read only this bundle. Original substantive artifacts are byte-identical copies.
No earlier ballots, candidate rationale, proposals, goal document, or other runs are included.

## Required evidence
- `task.txt`: exact worker acceptance request; template bound at spawn, no selection preamble required.
- `evidence/RUBRIC.md`: frozen scoring/gates; `evidence/acceptance.json` and `evidence/oracle.py`: frozen contract/oracle.
- `evidence/assigned-template.json`: actual assigned template, corroborate chronological guidance.
- `fixture-before/`, `fixture-after/`, `evidence/changes.json`, `evidence/diff.txt`: retained snapshots and scope.
- `transcript/messages.json`, `transcript/page-*.json`: raw committed transcript; `transcript/chronological.txt`: line-addressable rendering with message IDs/ordinals.
- `evidence/transcript-index.json`, `evidence/worker-final.txt`: message index and worker final response.
- `evidence/final-oracle.json`, `evidence/final-suite.json`, `evidence/pristine-with-worker-tests.json`: evaluator command outcomes, with matching copy/allocation records.
- `evidence/replay-selection.json`, `evidence/status.json`, `transcript/archive-summary.json`: limitations and completeness.
- `run-context.json`: sanitized execution context and deviations.
- `bundle-index.json`: all bundled files and byte hashes; `source-map.json`: rebased original inventory with hashes.

## Interpretation / limitations
Pristine overlay adds worker tests but not the new helper outside tests; its import error is a replay limitation, not an automatic gate. Judge chronological pre-change evidence independently. Evaluator checks corroborate artifacts, not unseen worker execution.
Parent reports archive/final bytes verified and container cleanup completed after capture; capture status reflects an earlier point. No live replay or container access is needed or allowed.
Independent sessions use the same model family. Condition blinding is imperfect: raw paths/metadata can disclose labels; substantive evidence was not altered. Ignore condition labels and judge this run alone.
Committed history is not a live event stream; inspect archive warnings and raw tool fields for material loss. Metrics not captured remain unavailable.

## Original inventory mapping
- `fixture-before/README.md` — original SHA-256 `34a039bf0bac5178aee89dd3ded393cbb60e670c958572bbe55201b67e4aed48`, 276 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-before/inventory.py` — original SHA-256 `1e9aa27356229fcd38b0d3e8053ca3fbbf9de3bc9a55a84ec077880d2eecd325`, 379 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-before/tests/test_smoke.py` — original SHA-256 `ddaceac728705c049f83bec6403917267279c079bfd333f2259c57c81d7bb314`, 182 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-before/notes/operator-draft.txt` — original SHA-256 `d46939cae5e3e620ee9350a64ceeaf46d126933427d77883c3beaa50270d69ee`, 50 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/README.md` — original SHA-256 `34a039bf0bac5178aee89dd3ded393cbb60e670c958572bbe55201b67e4aed48`, 276 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/classification.py` — original SHA-256 `91b76b92d1c9037a2fe4f2f7517441b810ceed2cf00558004dfc32097dbbaca2`, 296 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/inventory.py` — original SHA-256 `57e26da6f8dab15905f5782046d5914dad8154c5212d3f6b45b96bc327569d2c`, 235 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/__pycache__/classification.cpython-313.pyc` — original SHA-256 `e242497fd7892e9d5828dfe121d126f52899dc45c78d0cf85cd0bfa15fcccdcb`, 523 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/__pycache__/inventory.cpython-313.pyc` — original SHA-256 `931e9271c42a437e7a608c8364a9b1e81f7cf5eb9ca2caef564822bc85b7105b`, 494 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/notes/operator-draft.txt` — original SHA-256 `d46939cae5e3e620ee9350a64ceeaf46d126933427d77883c3beaa50270d69ee`, 50 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/tests/test_classification.py` — original SHA-256 `f4b9cfa41c0d8818d18ba6dbb9acec892ea03fdf5de8ec0678fc1e96894ee15b`, 1878 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/tests/test_smoke.py` — original SHA-256 `ddaceac728705c049f83bec6403917267279c079bfd333f2259c57c81d7bb314`, 182 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/tests/__pycache__/test_classification.cpython-313.pyc` — original SHA-256 `5e95ee28bc4650c612859cf665a9a6aa7d60556a121df5ff34d5d7dc64abf2c2`, 2921 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/tests/__pycache__/test_smoke.cpython-313.pyc` — original SHA-256 `c5127e8fa0ac4c04fd2f06fac7cb8bc2b450e13b3566149cb27eea29358180f3`, 729 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/get_state.json` — original SHA-256 `cea252832ea0ebb3d515956106a8e60e3dac7306c8f0dcb47eabe0cd0e0c2b34`, 594 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/get_session_stats.json` — original SHA-256 `a44364d83358d8ec615e901d54fb17a0536406e47a1869e1af0aca9edc424aca`, 590 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/page-0000.json` — original SHA-256 `a6045479c54683af8bd9d4f16c1f797a9133e04cb8871bdc5afe3865a1322a5a`, 12668 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/page-0001.json` — original SHA-256 `6bbd91da9a2cbbab8682c2d2a3aaaaee8f5175819d74efde3efbfef7c0a693da`, 12257 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/messages.json` — original SHA-256 `559d9b47f2955f2287e35c68ed1c4f9ac52b203c321d96ccd62f3c6c5973ce83`, 22353 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/archive-summary.json` — original SHA-256 `41992fd21aa2901213e4621c26d97a36945983e105aff1ff5ab303f61249bca7`, 248 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/status.json` — original SHA-256 `29d901a32137340a6deda6cd2ca2458762e0b4d1a1608789656b078ded05e89c`, 1053 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/archive.json` — original SHA-256 `2e00829adbdcc55b7b0fe537695c1044e506ec034be6af3ae5646757abf48c26`, 532 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/copy-fixture.json` — original SHA-256 `cf83566074402ba146245486ce4346b2dc382e409390b8035359066be6d17b3f`, 284 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/before-files.json` — original SHA-256 `68cbe9520be7c1b5a160a804f6eec015ff7933da99a27863a0ab09865f3ccca0`, 594 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/after-files.json` — original SHA-256 `8c98257d0a8aac190c1ef694f072463d71bec259dfbbf98b68e4ab01d8eee127`, 1609 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/changes.json` — original SHA-256 `2cd3259b076ca6775c05c405f6aefb55f3668ebabc1faa0cc4a3c304241c6340`, 244 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/diff.txt` — original SHA-256 `74c70a1bd7b2f95dd1cc16bc4a159c6ed6a38e4fd6889f02afe609d0ef7eea47`, 2972 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/acceptance.json` — original SHA-256 `acfa197c8a5aeb582d8d92c0b0491672bf82297646fd14da7fb417e14c9e82af`, 470 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/oracle.py` — original SHA-256 `11686394df19c3da267bd2c8dc8372723ee11f83d6a9f186fb3fba2d026d34be`, 634 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/RUBRIC.md` — original SHA-256 `e28b264ad670da1fa63dfa53700d1705956886265ee54e51d3825f0c54f16b1b`, 15148 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/assigned-template.json` — original SHA-256 `723667465ae91f8f65af3a086733a318c35f78b7ad27d55b18fa8966371f48b1`, 1234 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/transcript-index.json` — original SHA-256 `b58b136e1890e34cc4eeb147899c651e6e134d17c0a837c94e00ce62c736ebe5`, 6155 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/worker-final.txt` — original SHA-256 `359c2eeffa6b5213099901270ece03e22bda82662ef954c56b6cfa411172d797`, 533 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/allocate-stage.json` — original SHA-256 `00f2555ec145f0a03c0c22316082f678040079b254d3ff28eb89f46d2a1c8463`, 238 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/final-oracle-copy.json` — original SHA-256 `a4e294a13a2ba58ccbcf618c4a24a8627eaac2adac89a6f58b5f6d802d139db2`, 285 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/final-oracle.json` — original SHA-256 `3ca5d15c458eed0623ca7264d49ab45f1f6012638970d1803aeb2b60ffffa221`, 1178 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/final-suite-copy.json` — original SHA-256 `d18aea77710da047b9d9c7493490380646176f7a601895025d629f23296229df`, 284 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/final-suite.json` — original SHA-256 `4fc074cfbd29989dedf63742fa2d77bc4d1b87b7fdaca2453cd80f3cbc62d679`, 899 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/replay-selection.json` — original SHA-256 `3d88f50b87b52f94e28129e292764f90e8ef51725062c38cbfa51875f5107753`, 382 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/pristine-with-worker-tests-copy.json` — original SHA-256 `5ac0f29cc8326c25af18ac0e12d5c9755606278fb7bf2978381c63e2867b3cfc`, 254 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/pristine-with-worker-tests.json` — original SHA-256 `a611074b8715671418b67a1356d2685c4fc8ebab337399781403e94abc38695d`, 1520 bytes (same relative path, with capture directory rebased to evidence/).
- `task.txt` — original SHA-256 `d57f70f8c237e9fafc9b136261ae07387d427f547a4a6d64a2cbe9685462060b`, 424 bytes (same relative path, with capture directory rebased to evidence/).
