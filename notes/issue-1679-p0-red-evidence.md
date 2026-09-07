# P0 characterization falsifiability

Commands run at eb3c49d8 plus this uncommitted test change; no production implementation. Target `cargo test -p quecto-agentic-harness --test inference_admission_characterization <filter>`.

|Behavior oracle|Temporary boundary mutation|RED evidence|Restored GREEN|
|---|---|---|---|
|first delta identity/order|fake provider content first→wrong|`/tmp/1679-red/delta.log`, assertion first!=wrong, 1 test failed|4/4 suite|
|terminal only after release|withhold release channel indefinitely (typed channel)|`terminal.log`, bounded timeout, 1 failed|4/4|
|HTTP rejection classification|fake HTTP 429→500|`status.log`, HTTP429 expectation failed, 1 failed|4/4|
|more-than-buffer lossless delivery/terminal accumulation|fake source emits129 instead of130 deltas|`loss.log`, early Done at delta boundary, 1 failed|4/4|
|receiver drop alone lacks current termination ack|temporarily add `tx.closed()` select to real shared SSE pump|`drop-closes-transport.log`, actual socket EOF violates expected current behavior, 1 failed|4/4|
|refresh is a separate actual attempt|factory uses separate pre-incremented counter (same successful result)|`refresh-count.log`, actual-count2 assertion sees1, 1 failed|1/1 refresh target|

Intermediate invalid terminal-line mutation stayed GREEN because existing parser completes at EOF: discarded as inadequate, replaced with withheld terminal boundary. Initial untyped release mutation compile error discarded; rerun with typed channel fails behaviorally. Server task abort mutation only tested cleanup, not cancellation contract; replaced by real pump closure mutation. All production mutations restored (git diff confirms no sse_common change). Task guards abort on assertion unwind; all fake operations bounded. Auxiliary fixture-sanity assertions (header-size, joins, EOF/no-extra-events) share boundary failures and are not additional product behaviors. Additional per-assertion terminal/EOF falsifiers follow before final review.

Additional real-parser falsifiers: corrupt terminal accumulated content (`terminal-content.log`) fails both completion-content assertions; inject event after Done (`extra-terminal.log`) fails EOF assertions. Both run entire 4-test target and restore 4/4 GREEN. Transport agent independently recorded 21 named assertion mutations in `/tmp/1679-transport-mutation-evidence.json`, corresponding RED logs and restored 2/2 GREEN. Transport evidence is prototype-only; policy/status selection belongs to the fixture.

Local review hardening: production channel64→1024 mutation fails max_capacity oracle (`channel-bound.log`); reverse indexed delta source fails first ordered delivery (`delta-order.log`). Bound is output channel, not remote/network buffering. Revised fixture indexes deltas rather than indistinguishable x. Both behavioral mutations restored; new GREEN required below.

Transport mechanism falsifiers supplement inverted-oracle coverage: bridge replaces actual nested request with wrong envelope (`bridge-drops-request.log`) fails exact broker correlation; bridge connects to missing endpoint (`bridge-wrong-endpoint.log`) fails bounded real connectivity, not an echoed authority comparison. Both restored; final transport2/2 GREEN. No Python fixture residue.
