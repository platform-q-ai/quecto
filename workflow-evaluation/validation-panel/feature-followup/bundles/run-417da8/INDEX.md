# Evidence bundle: run-417da8

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
Pristine overlay is post-run sensitivity corroboration, not proof of chronological worker pre-change verification. Judge the chronological transcript independently. Evaluator checks corroborate artifacts, not unseen worker execution.
Parent reports archive/final bytes verified and container cleanup completed after capture; capture status reflects an earlier point. No live replay or container access is needed or allowed.
Independent sessions use the same model family. Condition blinding is imperfect: raw paths/metadata can disclose labels; substantive evidence was not altered. Ignore condition labels and judge this run alone.
Committed history is not a live event stream; inspect archive warnings and raw tool fields for material loss. Metrics not captured remain unavailable.

## Original inventory mapping
- `fixture-before/README.md` — original SHA-256 `702e70bcf3d870b2826f3da05b0216b7888646b8e0fbf3be917f967f55c28b80`, 224 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-before/search.py` — original SHA-256 `14a051a5876f9ff9a5faeba9806012181489471fc91e87e450bcf999d064b365`, 119 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-before/tests/test_smoke.py` — original SHA-256 `effc6bd32132defca8ae109a5631aa9046f42ad602cbfe51cfcde20cde5479c7`, 185 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-before/notes/operator-draft.txt` — original SHA-256 `d46939cae5e3e620ee9350a64ceeaf46d126933427d77883c3beaa50270d69ee`, 50 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/README.md` — original SHA-256 `702e70bcf3d870b2826f3da05b0216b7888646b8e0fbf3be917f967f55c28b80`, 224 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/search.py` — original SHA-256 `4eb9995f7d30c26da3e7d8612fc203e9c898bd178c670966593fa2438371985d`, 413 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/__pycache__/search.cpython-313.pyc` — original SHA-256 `35a2c18190f94a5bb507d23053cf6dad1ed36907e76ddfbd200299661a9eac57`, 697 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/notes/operator-draft.txt` — original SHA-256 `d46939cae5e3e620ee9350a64ceeaf46d126933427d77883c3beaa50270d69ee`, 50 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/tests/test_search.py` — original SHA-256 `2ed15918bd69faad02c09d4c05e40264fdea72ac91210f9adbce84d69066eaa6`, 2080 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/tests/test_smoke.py` — original SHA-256 `effc6bd32132defca8ae109a5631aa9046f42ad602cbfe51cfcde20cde5479c7`, 185 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/tests/__pycache__/test_search.cpython-313.pyc` — original SHA-256 `f06bbbf8d3dc7a09337c616ba28908ba289f3f94e1d30a45da603560f1126de3`, 3899 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/tests/__pycache__/test_smoke.cpython-313.pyc` — original SHA-256 `a499607bc1561ac2e58e23def1cdb549adb854f339ca4ae08baa48b0d88c6a28`, 726 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/get_state.json` — original SHA-256 `6a2ac7c63087e7a834f0e5555d146702a9e8e07c221d01afbaf75c9029f418a4`, 593 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/get_session_stats.json` — original SHA-256 `bcb8c9e9a49dcb2dceb091fb9c0ea9d34b1a74ceb49654bd9e07b10eaf9802a2`, 590 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/page-0000.json` — original SHA-256 `96ed05ccdf94bafe7fef34809f1fd9db7a2f67451a4e2a9b544769a9d912211f`, 16204 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/page-0001.json` — original SHA-256 `0b516a0656008e22867dbb8a3655af68f9d4404fd61ec48326d71a0ae595a7e1`, 13431 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/page-0002.json` — original SHA-256 `8ba4582cfecd574ba495dea80ac77406a2c3f08f26d231b82ae495323f1f1b13`, 625 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/messages.json` — original SHA-256 `862805037a49a754aa9038c3c3bba339e54c69707509b75c6f9d6230fb07a510`, 27262 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/archive-summary.json` — original SHA-256 `c306678a516ba7b1998547f7901e1988326e7b811b68946e74fdf57fbb932ecb`, 248 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/status.json` — original SHA-256 `ed6f840ebc028b15be2227e5f32fb8f014be5f6c93556dd5fc4e7f459db1db42`, 1054 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/archive.json` — original SHA-256 `f3ce86876d866b634bf052d88e58009a5fafb9dbd7977ffef74468baea77e65d`, 534 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/copy-fixture.json` — original SHA-256 `b6ada4f7550937be215bb5b0d47c10fa45dd3bb5164064340bb5e5dad726982d`, 285 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/before-files.json` — original SHA-256 `fa95a18b3be03f87c2764e0caad8dd633f9151f6fe11dd704452e9c92c865ff4`, 591 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/after-files.json` — original SHA-256 `2f29356d9acbe49f4c9b08af984b0730c87927dbac825fc65c91892e846269fd`, 1264 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/changes.json` — original SHA-256 `58f3b8d1e98dd9579f6f107d6645930e0432270cfa28879345c6a47c52b51291`, 208 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/diff.txt` — original SHA-256 `64ca4a1b82b26a17358b9cfe39d45c1185f618a4d1e2ff7d1938efe82c354db2`, 2788 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/acceptance.json` — original SHA-256 `9c46f699cbd14fca0606b8a7a6d84b97f4dc838cd6d15d55d196dbb38fa2beea`, 435 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/oracle.py` — original SHA-256 `c18cd30e9a5e2ff7207b524d6f9dc0fff4201392b214b676a16758266faa6dfb`, 793 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/RUBRIC.md` — original SHA-256 `e28b264ad670da1fa63dfa53700d1705956886265ee54e51d3825f0c54f16b1b`, 15148 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/assigned-template.json` — original SHA-256 `28a9cc688ab6e6b2dedc51b6913abb06d29f8ac0dcbfb0b167d176bb5200f2d3`, 2176 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/transcript-index.json` — original SHA-256 `dff9e87e3c286b0b1133104486ca7623a7c93d40d4e010155ec0bee9365fbffd`, 6478 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/worker-final.txt` — original SHA-256 `782201261aeded0fe086096b095fcdf237868e76577b50ca20e9a71cde347c08`, 388 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/allocate-stage.json` — original SHA-256 `d9d81fd0b395c399ff465d3ba21b7994485b312b8554cb9065691027f176c8ab`, 238 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/final-oracle-copy.json` — original SHA-256 `69c4bd905a8dbd1f6f529efc485c4c5ec955ea8d1cad460e8de96d3c5bead85f`, 286 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/final-oracle.json` — original SHA-256 `c0709a12f427a81fdde354b795a4c98adc8f066778c012aff46c1c876043b682`, 1344 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/final-suite-copy.json` — original SHA-256 `ae6acc76655b739178908e1c793f74829300008ae6c315606a78f8f1d515dd90`, 285 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/final-suite.json` — original SHA-256 `e302fed375e1cfd9120040742d3eaadd1d96ba9e6d203dc0c8504628d7bca686`, 1276 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/replay-selection.json` — original SHA-256 `0566a183db828a72dda6c0da51c5f037e51b7d61d010f73f01cf83b909b86e4b`, 374 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/pristine-with-worker-tests-copy.json` — original SHA-256 `1f34291e29429158ec6052f3c02acd8cea859cb2b91113366cd6126baf058436`, 254 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/pristine-with-worker-tests.json` — original SHA-256 `3f882da253f5aba10527b2ae0dec066c3f24c66a8e0f328c785fd259982f1f29`, 12477 bytes (same relative path, with capture directory rebased to evidence/).
- `task.txt` — original SHA-256 `6c0dbf4f70323a199b0d31952ee3b65fe17a5cb13c98499fefdb097515f4764b`, 439 bytes (same relative path, with capture directory rebased to evidence/).
