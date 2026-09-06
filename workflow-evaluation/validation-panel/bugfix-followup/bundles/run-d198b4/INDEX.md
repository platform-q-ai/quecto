# Evidence bundle: run-d198b4

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
- `fixture-before/README.md` — original SHA-256 `d372369b4fa0b32275bc7884e4aa877f3f38fc98419fc178f945c99202463985`, 244 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-before/intervals.py` — original SHA-256 `8dc04d06fbb6f03436463d0e171c15af31a2a41a8f8cfc2eb9f66ae968bbc2a1`, 283 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-before/tests/test_smoke.py` — original SHA-256 `b5b856e31bc0ef041c15a4783b4f841c746015ab8d1ba052d2fcbdfcb7b7dc43`, 164 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-before/notes/operator-draft.txt` — original SHA-256 `d46939cae5e3e620ee9350a64ceeaf46d126933427d77883c3beaa50270d69ee`, 50 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/README.md` — original SHA-256 `d372369b4fa0b32275bc7884e4aa877f3f38fc98419fc178f945c99202463985`, 244 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/intervals.py` — original SHA-256 `24a2bb60a5553e84a1858cf1545d53650f93f397dde38b61a384d4b461523d63`, 282 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/notes/operator-draft.txt` — original SHA-256 `d46939cae5e3e620ee9350a64ceeaf46d126933427d77883c3beaa50270d69ee`, 50 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/tests/test_merge.py` — original SHA-256 `600629b571853f7d4f06f78b4de4bb1dbdea655acf957d897e1ef1a6dbfb1b2f`, 908 bytes (same relative path, with capture directory rebased to evidence/).
- `fixture-after/tests/test_smoke.py` — original SHA-256 `b5b856e31bc0ef041c15a4783b4f841c746015ab8d1ba052d2fcbdfcb7b7dc43`, 164 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/get_state.json` — original SHA-256 `2e21aa2738c8015327efc3fc2feeefabeee5f80b4f7075ea1515748b475c5c1e`, 592 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/get_session_stats.json` — original SHA-256 `8a60181f02b6c6640a78c29872077ffb5618a650ef110e85b9c7b5835c423c0a`, 590 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/page-0000.json` — original SHA-256 `0447342247d7283920f56c425d49bfd7d9686c0da07a5d1481752da424164d53`, 17059 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/page-0001.json` — original SHA-256 `6eb868d8ef91b3d79c1edcd28959af0ade626002e401194807c4d481a16a642d`, 14123 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/messages.json` — original SHA-256 `b280f965c30599fc053ccaa137b28bddc86a6a90ba4420e8da5e67b03fd4de72`, 28570 bytes (same relative path, with capture directory rebased to evidence/).
- `transcript/archive-summary.json` — original SHA-256 `a755fb3eb621c9e6344f727e62a699a37cae72b4a786f192cfb72251afe7cf3d`, 248 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/status.json` — original SHA-256 `565c1c964f58a5639d403022f9f329d8e7bc60818114d96a3cef4ecc8a86a2b7`, 1053 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/archive.json` — original SHA-256 `9de546157417e981fe6fdfdccd0b1303c865954bb6c9f9fefad73537d284cad6`, 532 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/copy-fixture.json` — original SHA-256 `e9a3feafd510984e8bc1d85e70262f62b15562594fff328f3e90b63f4ac4ea87`, 284 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/before-files.json` — original SHA-256 `df8fa0169db6eeae96c22fe54b7b5ca6b311052322b04050c4f5698bdfdda411`, 594 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/after-files.json` — original SHA-256 `5d59845edb28a48d66ee94976489664353a15de6457e62947a6ccf3c7fa31bc5`, 745 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/changes.json` — original SHA-256 `c003484d7a5cd61cef978ea058218d0f5376c55d2444e60b50b8b55a01f9516f`, 210 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/diff.txt` — original SHA-256 `ebbdb7db574e56601cfa2ba21db7d1b3eb38d04358810151638219346e4951ca`, 1354 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/acceptance.json` — original SHA-256 `f12511c1fca92610f48c050b90d8c5a8357189ae8489365b4f4f0b3eebbcfcd7`, 466 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/oracle.py` — original SHA-256 `a28053fe3ba51f4123889a2261ef03eab2bc1600ee7926e991b04440386adab6`, 488 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/RUBRIC.md` — original SHA-256 `e28b264ad670da1fa63dfa53700d1705956886265ee54e51d3825f0c54f16b1b`, 15148 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/assigned-template.json` — original SHA-256 `cb93cc54770b49595f707dec30eefcef4e8e3d95cd7bb426b3f9f354f60f2859`, 2341 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/transcript-index.json` — original SHA-256 `5d24f7b52d9efe52f16cf2b3ebb6082b2fa065354301ed070c1e7bfdb1b507cf`, 6325 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/worker-final.txt` — original SHA-256 `5fefac799374a80e49ab031864bb7081a02425d0ca0fdd25751b4b87dea64a32`, 376 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/allocate-stage.json` — original SHA-256 `d2bf8f7110cc0c71dd30818cd5cc373dcdf49c77181feb836ed5c940030e4f7f`, 238 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/final-oracle-copy.json` — original SHA-256 `9e29f746dd8b565a22651052814c9c11f5f402dbd8af709972a1f6139370e3a9`, 285 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/final-oracle.json` — original SHA-256 `7e4e65b1ed858f2658fe66f2f2181bc7c43463798ed853e97c08ec2c5cbc61db`, 946 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/final-suite-copy.json` — original SHA-256 `fb86d758dc84143c5dd1f8c82683ac79d72db7a049e63aac4362228d96cdeb06`, 284 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/final-suite.json` — original SHA-256 `69897b4558465aeeaf8beaee5cb6f4657a55ce4cf15e06d244075afbdc891a0d`, 1246 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/replay-selection.json` — original SHA-256 `16b9263bafd1f7f4536a8896d13b2e22c6b65d2633efcc954c7b3093caad68f9`, 373 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/pristine-with-worker-tests-copy.json` — original SHA-256 `abf47362671dc8c7e182b46411bed631433b393825a2d8ec2f00489dfcd78b76`, 254 bytes (same relative path, with capture directory rebased to evidence/).
- `evidence/pristine-with-worker-tests.json` — original SHA-256 `67d8b3f9bbb84de804463bd24acbf5999c5a5bcb4373626a2a971f2933037e96`, 2927 bytes (same relative path, with capture directory rebased to evidence/).
- `task.txt` — original SHA-256 `163ffbbcefde081f0d77b587c04420728c8f1b6603ce28922717f3b0adf09f40`, 326 bytes (same relative path, with capture directory rebased to evidence/).
