# #1679 P4 slice 3 — presentation, AC matrix, measured comparison and rollout

Scope (notes/1679-p4-scope.md item 3), stacked on slices 1 (#1700) and 2 (#1701).

## Delivered

- **TUI** (`quecto-tui`): `protocol/admission_payloads.rs` (bounded, sanitized
  `AdmissionView` with `status_label`/`compact_label`), `get_state.admission` on
  the state snapshot, `Event::AdmissionStateChanged { agent_id, admission }`,
  master footer label ("⏳ waiting for admission 12s · anthropic cooldown 30s")
  and working-spinner message, forwarded descendant label on the panel row
  (truncated before the identity is), waiting label dropped at run end while a
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
