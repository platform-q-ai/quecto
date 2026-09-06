# Evidence bundle: run-82f6d0

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
Pristine overlay is post-run parity corroboration, not proof of chronological worker pre-change verification. Judge the chronological transcript independently. Evaluator checks corroborate artifacts, not unseen worker execution.
Parent reports archive/final bytes verified and container cleanup completed after capture; capture status reflects an earlier point. No live replay or container access is needed or allowed.
Independent sessions use the same model family. Condition blinding is imperfect: raw paths/metadata can disclose labels; substantive evidence was not altered. Ignore condition labels and judge this run alone.
Committed history is not a live event stream; inspect archive warnings and raw tool fields for material loss. Metrics not captured remain unavailable.

## Original inventory mapping
- `fixture-before/README.md` — original SHA-256 `f7366e58c279f2344fa8d386e88b225368e559db1778d1eef41fa8aa5b9e694b`, 262 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-before/access.py` — original SHA-256 `9f9fde04b1c7eaefbff77994c422885b950e9d535c2d9834040b43c5b52e86ff`, 384 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-before/tests/test_smoke.py` — original SHA-256 `381f653a4c960da8259fd3a7f9b5c5d08c79d367ca7d4971d309cdccd28636db`, 172 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-before/notes/operator-draft.txt` — original SHA-256 `d46939cae5e3e620ee9350a64ceeaf46d126933427d77883c3beaa50270d69ee`, 50 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/README.md` — original SHA-256 `f7366e58c279f2344fa8d386e88b225368e559db1778d1eef41fa8aa5b9e694b`, 262 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/access.py` — original SHA-256 `9382c00e0ee5f9ec3f5064fab541d13e676b92557f77e84b8d5b4a8176ad5c11`, 256 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/__pycache__/access.cpython-313.pyc` — original SHA-256 `67d8ed284a2bc6835b6e7fe8b97ab7c7393d313713d9a45a9528ecaa8cc96b7a`, 390 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/notes/operator-draft.txt` — original SHA-256 `d46939cae5e3e620ee9350a64ceeaf46d126933427d77883c3beaa50270d69ee`, 50 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/tests/test_parity.py` — original SHA-256 `8aee5bc68c65edec075d823084537d2f598fd4ce9e87358eb3662889d5ef92d7`, 1048 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/tests/test_smoke.py` — original SHA-256 `381f653a4c960da8259fd3a7f9b5c5d08c79d367ca7d4971d309cdccd28636db`, 172 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/tests/__pycache__/test_parity.cpython-313.pyc` — original SHA-256 `a4d4e54322ff592d6bde97d654460e1dedcf8747f33f4545415773cc2509ed92`, 1176 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/tests/__pycache__/test_smoke.cpython-313.pyc` — original SHA-256 `5c2792eab120ba2cdc86af7d33c39f560363e3241449fe1fe2aa0611e8e8c149`, 694 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/get_state.json` — original SHA-256 `e9a9a25156b5f99d5ee26275714195ff1607b110da390e7ebdc66886daf7eb6d`, 594 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/get_session_stats.json` — original SHA-256 `368a6774c29a9c45046ce2e97db33ee69ce7abed24363773023dad36061a8171`, 590 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/page-0000.json` — original SHA-256 `3befec9fb487e9ce0b730aff4587c87dd61bcc84838fbe6ca4a2840330714829`, 12953 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/page-0001.json` — original SHA-256 `f93b74665d4cea5c2268bf262e6697063569c990c2b6de42694d5a271a4a8a20`, 19191 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/page-0002.json` — original SHA-256 `4e3b92e745117c2b2c39c84a4db2bab6833846087645570761101dabe47ca9db`, 625 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/messages.json` — original SHA-256 `d59f6d779efd702a46c4c4ced4a701fcf1dff07dfc0b52943adc43795e127ff4`, 29819 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/archive-summary.json` — original SHA-256 `c306678a516ba7b1998547f7901e1988326e7b811b68946e74fdf57fbb932ecb`, 248 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/status.json` — original SHA-256 `61d4996b86f78f2e709a6b1a2b4756153e0396c6eab268b1650875381c0b76d3`, 1053 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/archive.json` — original SHA-256 `80d4764aeb0ea201c9129050bc4860bddd9b9791b6c1801709ba0f037509c7f1`, 532 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/copy-fixture.json` — original SHA-256 `65d20a5fd2cdfb3f81179de9e3c56820059253b8407fbd87fb82724ac0f82e8a`, 284 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/before-files.json` — original SHA-256 `379c8ebf80fa2f4d132a455128872db6422fadc83a7b810c4735dd4fa4f723f4`, 591 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/after-files.json` — original SHA-256 `f9e3a7101838515766c9bb7a14f5c5ec79f174abefa2fcc79a7cd3f91dcdc41a`, 1264 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/changes.json` — original SHA-256 `fa9f7d8da338e07bf210ce2ac0ad60024607bc40abbe2f46e2b8cf827694f402`, 208 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/diff.txt` — original SHA-256 `c5f7ce12e533b66ea3f01637d784b4b33d4ccae84b9f6238c870e0ca110bdc4d`, 1805 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/acceptance.json` — original SHA-256 `e333a97b4b2dcb5959a35d8f8b716025be9dbde8f1178f881d818e840795ecb7`, 445 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/oracle.py` — original SHA-256 `3c21a12e6b23ca3ab23a6259f52e3b4b6b0790f9d43acd28892b269688eca67c`, 596 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/RUBRIC.md` — original SHA-256 `e28b264ad670da1fa63dfa53700d1705956886265ee54e51d3825f0c54f16b1b`, 15148 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/assigned-template.json` — original SHA-256 `723667465ae91f8f65af3a086733a318c35f78b7ad27d55b18fa8966371f48b1`, 1234 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/transcript-index.json` — original SHA-256 `a6eef8f9d663c74e814894c3b1301134f17913048f355678da494149931e40d2`, 6474 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/worker-final.txt` — original SHA-256 `0c7609b947212d713850f8eaa47d828ae9c2093dabf589944e438d98dd32c4fe`, 390 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/allocate-stage.json` — original SHA-256 `c632a6aaa76915519f9f0ce0158170f01a5ecd26da901d00e53727954d17ff28`, 238 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/final-oracle-copy.json` — original SHA-256 `d91f993133b0b2fdc8b1f1572779a1682b87a1ce8d376792956e56abf56ee400`, 285 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/final-oracle.json` — original SHA-256 `7dc5e1a3e6cf0f8f84eb8c381bf27521e9dfe54a783f7ab3e3dcfcd03e5d576c`, 1117 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/final-suite-copy.json` — original SHA-256 `ccad06e5cbc5746988648215d030d6340e2e12dd2d1f1000ce16d77b9c8dc84d`, 284 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/final-suite.json` — original SHA-256 `f3837428b2dc89b1ebc3d7c6fa60a9c46872af6117aa4ad302dd993b55d5aeb4`, 701 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/replay-selection.json` — original SHA-256 `2d943ec5fa7ac4120adfb22ffdf5872122cda2f948dc510963140db2add25336`, 374 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/pristine-with-worker-tests-copy.json` — original SHA-256 `aab861d3c04e42585fe50e5f8d94a702fdee383bab2284b0f59b6f56b7c1f7f6`, 254 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/pristine-with-worker-tests.json` — original SHA-256 `3cf9300805250e05bbc6fdd9ade72a87e31d90967fb3a42ceac42f03dee22ad8`, 716 bytes (same relative path, with capture directory rebased to evidence/).
- `task.txt` — original SHA-256 `f7bf80b393ef459608f3006963ea08c0c9c29177ca2312e14033086f7a68e151`, 328 bytes (same relative path, with capture directory rebased to evidence/).
