# #1679 P4 slice 3 — presentation, AC matrix, measured comparison and rollout

Scope (notes/1679-p4-scope.md item 3), stacked on slices 1 (#1700) and 2 (#1701).

## Delivered

- **TUI** (`quecto-tui`): `protocol/admission_payloads.rs` (bounded, sanitized
  `AdmissionView` with `status_label`/`compact_label`), `get_state.admission` on
  the state snapshot, `Event::AdmissionStateChanged { agent_id, admission }`,
  master footer label ("⏳ waiting for admission 12s · anthropic cooldown 30s")
  and working-spinner message, forwarded descendant label on the panel row
  (all or nothing, never a fragment), waiting label dropped at run end while a
  cooldown survives. No lifecycle state is touched (`running`, roster status).
- **Harness**: the parent's monitor forwards a child's `admission_state_changed`
  re-stamped with `agent_id`/`parent_id`, rebuilt from known fields only (groups
  bounded to 32, oversized frames dropped whole); protocol docs updated.
- **Integrated AC matrix** (`inference_admission_matrix.feature`, real authority):
  round-robin across roots (a descendant cannot increase its root's share),
  interactive reserve kept free from background work, explicit queue-full and
  deadline refusals, authority outage fails closed within the bounded wait,
  pacing across independent roots.
- **Measured fake-workload comparison** (`burst_versus_admitted_fake_workload_is_measured`,
  four root processes, provider hold 600 ms, no paid traffic):

  |Run|peak concurrency|attempts|wall time|
  |---|---|---|---|
  |burst (no admission)|4|4|611 ms|
  |admitted, C=2|2|4|1 221 ms|

  Admission delayed, never dropped, attempts; the admitted run took two waves.
- **Runbook**: activation, rollback and quarantine in `docs/inference-admission.md`;
  ADR-0026 P4 status section.

## AC evidence matrix (P4 closure)

|AC|Evidence|
|---|---|
|AC1 shared bound|`inference_admission_authority.feature` (two roots, one slot; quarantine; reset epoch); `tests/inference_admission_processes.rs` (three roots bounded at C=2, descendant behind root, SIGKILL client/authority, control burst); measured comparison above; gated Podman proof `tests/inference_admission_container.rs`|
|AC2 fairness/headroom|policy contracts (`notes/issue-1679-p1-matrix.md`); integrated `inference_admission_matrix.feature` scenarios 1–2 (RR across roots with a descendant; background leaves reserve free)|
|AC3 pacing/identity|`inference_admission.feature` (pacing boundary, no refund), `inference_admission_operations.feature` (unknown alias refused, owner token, no self-promotion), runtime17 identity/reload tests (P2), matrix scenario 5 (pacing across roots)|
|AC4 attempt lifetime|`inference_admission_provider.feature` (receiver ≠ completion, no replay), `inference_admission.feature` (idle parent holds nothing; cancelled queue never dispatches), attempts65/retry2 (P2)|
|AC5 recovery|`inference_admission_http.feature` outline (Retry-After forms), provider feature (header advice blocks siblings), P2 feedback/receipt contracts, slice 1 cooldown observation (`inference_admission_observation.feature`)|
|AC6 failure safety|matrix scenarios 3–4 (queue full, deadline, outage fails closed), operations feature (corrupt/missing ledger fails closed, raced cancel), authority feature (vanished session quarantines)|
|AC7 observation/compatibility|`inference_admission_observation.feature`, `inference_admission_projection.feature` (waiting verdict, since cursor, abort while waiting, ordered push), `tui_admission_waiting.feature` (footer/spinner/panel, no lifecycle change), error-compatibility/characterization suites (P2), forwarded descendant events|
|AC8 verification/docs|architecture + contract gates, this note, `docs/inference-admission.md` (configuration, limitations, runbook), measured comparison, no paid traffic|

Review and mutation records are appended below.

## Adversarial review 1 (2026-09-09) and fixes

Twelve items (2 high, 4 medium, 6 low), all fixed:

|Finding|Fix|Proof|
|---|---|---|
|H1 four files over the 750-line cap|`extract_result_text` → `client_result_text.rs`; workflow-automation helpers → `app_workflow_automation.rs`; admission mods declared from `app_events.rs`; monitor forwarding through `bounded_forward`/`forward_child_admission_event` in the canonical module|quality gate|
|H2 child labels never pruned, insertable for any id (unbounded)|labels only for tracked children; pruned on every update|`a_forwarded_child_view_labels_a_tracked_child_only`|
|M1 master label survives disconnect / an agent without admission|cleared on disconnect and when `get_state` carries no view|`a_tool_spinner_message_is_never_clobbered_and_disconnect_clears_the_label`, `get_state_applies_or_clears_the_admission_view`|
|M2 forwarded identity spoofable (child could paint the parent's footer)|embedded `agent_id` honoured only for a registered descendant that is not the parent; otherwise stamped as the child|`forwarded_admission_identity_cannot_be_spoofed`|
|M3 admission clobbered tool spinner text|the module remembers the message it wrote and restores only over that|spinner test above|
|M4 pacing measured between call returns|measured from before the first request, provable 200 ms lower bound|`inference_admission_matrix.feature` scenario 5|
|L1 runbook named non-existent status fields|`"journal_healthy": true`, `"epoch": 1`|docs|
|L2 master panel row used the long label|compact label on both rows|`footer_shows_the_admission_label_beside_the_model_and_clears_it`|
|L3 partial label fragments in the panel|all-or-nothing label|panel render|
|L4 500 ms queue deadline could elapse before the third request|2 s deadline|matrix scenario 3|
|L5 run-end clearing keyed on the label text|re-derived from the last view with nothing waiting|"waiting-room" cooldown case in the master test|
|L6 stale "single scan" comment|comment states the two substring gates|—|

## Mutations (slice 3, round 1, after review 1)
    T1 waiting never labelled, T2 elapsed cooldown shown, T3 groups unbounded, T5 waiting label survives run end,
    T6 spinner not updated, T7 footer ignores admission, T9 unknown child id labelled, T10 labels never pruned,
    T11 spinner restored over a foreign message, T12 disconnect keeps the label, H1 forwarded groups unbounded,
    H4 spoofed identity honoured: all killed.
    T8 panel row drops child label: SURVIVED (BDD-only proof) → lib test added, re-run in round 2.
    H2/H3/T4: mis-anchored (workflow function / moved code), re-targeted in round 2.

## Adversarial review 2 (2026-09-09) and fixes

All twelve round-1 fixes verified. Eight new items, all fixed:

|Finding|Fix|Proof|
|---|---|---|
|M1 `agent_error` ended a run without clearing a waiting label|`clear_master_admission_wait` on agent error|`agent_error_ends_the_wait_and_the_panel_row_paints_a_child_label`|
|M2 a registered sibling could be painted by a child; claimed parent honoured|identity honoured only for a descendant reached by walking the registry's parent links from the embedded id to the emitting child; the parent is stamped from the monitor otherwise; docs state the rule|`forwarded_admission_identity_cannot_be_spoofed` (root, stranger, sibling, empty)|
|M3 substring gate swallowed lifecycle lines quoting the event name|gate on `"type":"admission_state_changed"` and fall through unless the forwarder matched|quoted `agent_start` case in the same test|
|L4 tool end wiped the wait from the spinner|tool end and run start reuse the remembered admission message|same test|
|L5 note/doc drift|note corrected; protocol doc carries the identity rule|—|
|I6 BDD scenario title vs body|cooldown reported before the run ends|`tui_admission_waiting.feature`|
|I7 spinner created after a stored wait is unlabelled|run start seeds the spinner from the stored message|—|
|I8 empty SGR pair; over-wide test-support visibility|dim only a non-empty label; visibility back to `pub(super)`|—|

Architecture ratchets (pre-push): the admission mapper deserializes a typed
`AdmissionView` DTO (no raw `serde_json` sites), `client_result_text.rs` is
allowlisted as a pure `client.rs` relocation, the wire-DTO total records the
one new dispatch arm, and the owner map lists the four new files.

## Mutations (slice 3, round 2, after review 2)
    T1–T14 (TUI: label logic, bounds, master/child routing, run-end/agent-error/disconnect clearing,
    spinner ownership incl. tool end, footer and panel row painting) and H1–H6 (harness: forwarded
    group bound, identity re-stamping, oversized drop, spoofed parent/sibling identity, quoted
    lifecycle line not swallowed): 25 mutants, all killed. Local gates: pre-push green, harness lib
    4295 / TUI lib 2168 green, admission lane 25/25, TUI lane 214/214, function coverage harness
    92.17% / TUI 95.32%.
