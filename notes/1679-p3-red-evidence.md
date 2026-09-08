# #1679 P3 RED evidence and mutation record

Branch `issue-1679-p3` (worktree `quecto-wt-1679`), base 6a383f30. Each slice
started from a failing test; per-assertion sensitivity was then proven by
deliberate production mutations (ADR-0007), restored with `git checkout`.

## Slice 1 — domain recovery (`tests/contracts/admission_recovery.rs`)
Initial RED: compile failure (missing `AdmissionRecovery`, `Quarantined`,
`GroupSnapshot.uncertain`, `abandon/withdraw/next_wake/ledger/reset/restore`),
28 errors. GREEN: 13 assertions + `high_water` added later (14).

## Slice 2 — authenticated authority (`tests/contracts/admission_journal.rs`,
`admission_secret_source.rs`)
Initial RED: unresolved `inference_authority`/ports. GREEN: 12 assertions.

## Slices 1–2 mutations (contracts `admission_` filter, 109 tests)
    === MUTANT M1 no quarantine check in next()
      test admission_journal::disconnect_cancels_queued_work_and_quarantines_active_work ... FAILED
      test admission_recovery::abandon_marks_active_attempt_uncertain_and_quarantines_its_group ... FAILED
      test admission_recovery::restore_turns_outstanding_work_into_orphaned_quarantine_until_reset ... FAILED
    === MUTANT M2 terminal keeps uncertain
      test admission_journal::disconnect_cancels_queued_work_and_quarantines_active_work ... FAILED
      test admission_recovery::verified_completion_by_the_same_scope_clears_quarantine ... FAILED
    === MUTANT M3 withdraw refunds pacing
      test admission_recovery::withdraw_terminates_an_undelivered_grant_without_refunding_pacing ... FAILED
    === MUTANT M4 reset drops cooldown
      test admission_recovery::reset_starts_a_successor_epoch_that_forgets_uncertainty_but_keeps_cooldown ... FAILED
    === MUTANT M5 restore ignores orphans
      test admission_recovery::restore_turns_outstanding_work_into_orphaned_quarantine_until_reset ... FAILED
    === MUTANT M6 next_wake ignores cooldown
      test admission_recovery::next_wake_reports_the_earliest_state_change ... FAILED
    === MUTANT M7 snapshot hides orphans
      test admission_recovery::restore_turns_outstanding_work_into_orphaned_quarantine_until_reset ... FAILED
    === MUTANT M8 pump grants without durable ledger
      test admission_journal::journal_failure_withdraws_the_grant_and_stops_new_grants_until_recovery ... FAILED
      test admission_journal::a_grant_is_journaled_before_it_becomes_visible ... FAILED
    === MUTANT M9 complete acknowledges without durable release
      test admission_journal::completion_persists_the_release_before_acknowledging ... FAILED
    === MUTANT M10 authenticate ignores token
    === MUTANT M11 reset keeps capabilities
      test admission_journal::reset_revokes_every_capability_and_starts_a_new_epoch ... FAILED
    === MUTANT M12 abandon leaves queued work dispatchable
      test admission_recovery::abandon_cancels_queued_work_so_it_never_dispatches ... FAILED
    === MUTANT M10b authenticate ignores token
    test admission_journal::child_registration_requires_the_parent_capability_and_inherits_its_root ... FAILED
    test admission_journal::roots_receive_distinct_secrets_and_forged_tokens_are_unauthorized ... FAILED
    test result: FAILED. 107 passed; 2 failed; 0 ignored; 0 measured; 163 filtered out; finished in 0.00s

M10 first attempt was a compile error (dead-code deny), rerun as M10b (`|| true`)
which killed two authentication assertions. All 12 mutants killed.

## Slice 3 — process adapters (`tests/inference_admission_broker.rs`)
Initial RED: unresolved `infrastructure::admission::{AdminConnection,
AuthorityConnection, AuthorityDirectory, AuthorityServer, ClientError,
FileJournal, SingletonLock}`. GREEN: 8 scenarios over real UDS/files.

## Slice 3 mutations (`inference_admission_broker`, 8 tests)
    === MUTANT N1 server ignores disconnect
      test connection_loss_quarantines_until_the_same_capability_reconciles ... FAILED
    === MUTANT N2 dropped acquire never cancels
    === MUTANT N3 corrupt ledger restarts empty
      test file_journal_is_durable_round_trip_and_fails_closed_on_corruption ... FAILED
    === MUTANT N4 shared directory accepted
      test directory_is_private_and_the_lock_is_a_singleton ... FAILED
    === MUTANT N5 restart ignores ledger
      test reset_revokes_capabilities_and_restart_orphans_outstanding_work ... FAILED
    === MUTANT N6 admin accepted on client socket
      test protocol_rejects_unsupported_hello_unbound_operations_and_child_admin ... FAILED
    === MUTANT N7 any protocol version accepted
      test protocol_rejects_unsupported_hello_unbound_operations_and_child_admin ... FAILED
    === MUTANT N8 reset keeps session capabilities
    === MUTANT N9 queued acquire granted immediately
      test journal_failure_denies_grants_instead_of_bypassing ... FAILED
      test dropping_a_queued_acquire_cancels_it_before_dispatch ... FAILED
      test hello_publishes_the_effective_policy_and_grants_release_in_order ... FAILED
      test reset_revokes_capabilities_and_restart_orphans_outstanding_work ... FAILED
      test connection_loss_quarantines_until_the_same_capability_reconciles ... FAILED
    === MUTANT N10 socket not chmod 0600 (expected: no test covers; informational)

N2 (dropped acquire never cancels) initially SURVIVED: the queued request
expired at the 5 s queue deadline instead. The test now uses a 60 s deadline
and a 1 s bound, after which N2 fails `dropping_a_queued_acquire...` (1.07 s).
N8 (server session keeps its credential after reset) is an equivalent mutant:
the authority's issued map is the only authentication source, so the client
still observes `Unauthorized`; the session field is defensive only.
N10 (socket chmod) failed to compile; socket mode is asserted nowhere yet —
noted as a follow-up assertion for the directory/socket permission test.

## Slice 4 — interface/launch (`tests/inference_admission_processes.rs`,
`config_admission_tests.rs`)
Initial RED: unresolved `write_admission_context`, missing `admission-broker`
subcommand, `--admission-context` flag and `admission` config section (compile
failure). GREEN: 5 real-process proofs (three roots at C=2 peak<=2/total 3;
control without admission peak 3; descendant waits behind root at C=1 and a
forged token exits non-zero with zero provider attempts; SIGKILLed client ->
uncertain=1 -> next root fails explicitly -> `admission-broker reset` epoch 2;
SIGKILLed authority -> restart keeps epoch, orphan active/uncertain 1, uds root
survives) plus 3 config assertions. Slice 4 mutations recorded below once run.

## Slice 4 mutations (`inference_admission_processes`, 5 real-process tests)
    === MUTANT P1 roots skip negotiation (bypass)
      test sigkilled_authority_restarts_with_orphans_and_does_not_kill_clients ... FAILED
      test independent_root_processes_share_the_configured_bound ... FAILED
      test sigkilled_client_leaves_uncertain_occupancy_until_operator_reset ... FAILED
      test result: FAILED. 2 passed; 3 failed; 0 ignored; 0 measured; 0 filtered out; finished in 30.06s
    === MUTANT P2 composition ignores installed binding
      test sigkilled_authority_restarts_with_orphans_and_does_not_kill_clients ... FAILED
      test independent_root_processes_share_the_configured_bound ... FAILED
      test descendant_context_waits_behind_its_root_and_a_forged_context_fails_closed ... FAILED
      test sigkilled_client_leaves_uncertain_occupancy_until_operator_reset ... FAILED
      test result: FAILED. 1 passed; 4 failed; 0 ignored; 0 measured; 0 filtered out; finished in 30.10s
    === MUTANT P3 child ignores bind failure
    === MUTANT P4 disconnect silently resets epoch
      test sigkilled_client_leaves_uncertain_occupancy_until_operator_reset ... FAILED
      test result: FAILED. 4 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 30.07s
    === MUTANT P5 restart forgets ledger
      error: function `decode` is never used
      error: could not compile `quecto-agentic-harness` (lib) due to 1 previous error
    === MUTANT P3b child ignores bind failure
    === MUTANT P5b restart forgets ledger
    test sigkilled_authority_restarts_with_orphans_and_does_not_kill_clients ... FAILED
    test result: FAILED. 4 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.10s
    === MUTANT P3c child ignores bind failure (readiness assertion)
    test descendant_context_waits_behind_its_root_and_a_forged_context_fails_closed ... FAILED
    test result: FAILED. 4 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.10s

P3 (child ignores a bind failure) initially SURVIVED: the one-shot forged child
still failed at its first acquire, after startup. The descendant test now also
launches a `--mode uds` child with the forged sidecar and asserts its control
socket never accepts before it exits non-zero; P3c then fails that test. P5 was
a dead-code compile error; P5b (`decode(..).map(|_| None)`) kills the authority
SIGKILL/restart test. All slice 4 mutants killed.

## Real container evidence (`tests/inference_admission_container.rs`)
Gated on `QUECTO_ADMISSION_CONTAINER_E2E=1`; run locally with podman and
`quecto-box:local` (2026-09-08): create.sh received `QUECTO_ADMISSION_DIR`,
bind-mounted the client directory and reported `shared-directory-v1`; the child
bound its capability through the mount before its control socket accepted; a
prompt queued behind the test root (active 1 / queued 1) and was granted only
after the root released; without the directory the adapter reports no
capability. 1 passed in 1.54 s.

## Adversarial review (2026-09-08) and RED for each fix

An independent adversarial review of the branch diff reported 4 high, 7 medium
and 6 low findings; every one was reproduced against the code and fixed with a
failing test first (`notes/1679-p3-verification.md` lists the observable rows).
Initial RED (compile-level) captured in `review-red.log`: missing
`cancel_detached`, `register_root_with_token`, `root_token_path`,
`start_accepting_missing_ledger`, `FRAME_DEADLINE`, `live_scopes`,
`retire_child`, and the `Result`-returning `admission_proposal`.

|Finding|Fix|Proof|
|---|---|---|
|H1 client `select!` dropped half-read frames|reader and writer on separate tasks|`interleaved_grants_and_completions_never_desynchronize_the_connection` (256 cycles, connection still bound)|
|H2 grant/cancel race leaked the slot|tracked cancel; an `Active` reply completes as failed|`cancel_after_grant_releases_the_slot_as_a_failed_attempt`|
|H3 no shutdown path; scopes never retired|completions on the connection runtime, bounded drain, `retire` at exit|processes: `live_scopes == 0` after roots exit|
|H4a default directory visible to containers|adapter masks the authority root with an empty dir, re-binds `client/`|container e2e: journal/admin/token absent, socket present, on podman|
|H4b any client could mint roots|owner token (0600, outside `client/`) required|`root_registration_requires_the_owner_token_outside_the_client_directory`|
|M1 concurrent acquires hit the replay fence|sequence allocated and sent under one lock|`concurrent_acquires_on_one_scope_are_never_rejected_as_replay`|
|M2 vanished ledger restarted empty|initial checkpoint; missing-after-operation refused unless acknowledged|`missing_ledger_after_prior_operation_fails_closed`|
|M3 rebind orphaned queued work|supersede disconnects the old session|`rebinding_a_capability_supersedes_the_previous_session`|
|M4 unknown completion poisoned all groups|refused without poisoning|`completing_a_never_enqueued_sequence_is_refused_without_poisoning_groups`|
|M5 scope exhaustion|live-scope limit, monotonic serials, parent retires unlaunched child|`scope_limit_counts_live_scopes...`, `a_parent_can_retire_only_its_own_descendants`|
|M6 create.sh leaked env dir|precondition before mktemp|container e2e: failed create leaves no env dir|
|M7 outage drained queue as "cancelled"|probe before selection, ledger error to withdrawn waiters, probe cadence|`unhealthy_journal_is_probed_before_dispatch...`, `journal_outage_holds_queued_work...`|
|L1 missed close wakeup|`Notified::enable` before the flag check|reviewed; existing close tests|
|L2 `inspect_epoch` workaround|ledger epoch reused|existing cancellation contract|
|L3 doc/`expect` panics|`admission_proposal` returns `Result`|`admission_proposal_reports_invalid_sections_instead_of_panicking`|
|L4 relative directory|absolute path required|`relative_authority_directory_is_rejected`|
|L5 sidecars never removed|child consumes after bind; failed launch removes|processes: sidecar absent after descendant run|
|L6 no framing deadline|15 s deadline once a prefix arrives; cap 256 KiB validated at start|`a_stalled_frame_is_disconnected_within_the_framing_deadline`|
|N1 restart needs agent restart|documented|docs|

P1-era contract expectations that encoded the replaced behaviour (retired
scopes counting toward the limit; unknown completion poisoning) were updated in
`admission_client.rs` with the new rationale. Mutation results for the fixes
are appended after the final run.

## Review-fix mutations (final run)
    === MUTANT R2 raced grant not completed on cancel [inference_admission_broker]
      test cancel_after_grant_releases_the_slot_as_a_failed_attempt ... FAILED
      test result: FAILED. 15 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 15.02s
    === MUTANT R4 owner token not checked [inference_admission_broker]
      test root_registration_requires_the_owner_token_outside_the_client_directory ... FAILED
      test result: FAILED. 15 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 15.02s
    === MUTANT R5 missing ledger silently accepted [inference_admission_broker]
      test missing_ledger_after_prior_operation_fails_closed ... FAILED
      test result: FAILED. 15 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 15.03s
    === MUTANT R6 rebind does not supersede [inference_admission_broker]
      test rebinding_a_capability_supersedes_the_previous_session ... FAILED
      test result: FAILED. 15 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 15.02s
    === MUTANT R7 no journal probe before selection [contracts]
      test admission_journal::unhealthy_journal_is_probed_before_dispatch_so_queued_work_is_held ... FAILED
      test admission_journal::journal_failure_withdraws_the_grant_and_stops_new_grants_until_recovery ... FAILED
      test result: FAILED. 275 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.05s
    === MUTANT R8 no framing deadline [inference_admission_broker]
      test a_stalled_frame_is_disconnected_within_the_framing_deadline ... FAILED
      test result: FAILED. 15 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 20.02s
    === MUTANT R9 scope limit counts retired scopes [contracts]
      test admission_client::duplicate_and_conflicting_acquire_terminal_eviction_and_scope_limits ... FAILED
      test admission_recovery::scope_limit_counts_live_scopes_and_serials_are_never_recycled ... FAILED
      test result: FAILED. 275 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.04s
    === MUTANT R10 shutdown does not retire [inference_admission_processes]
      test independent_root_processes_share_the_configured_bound ... FAILED
      test result: FAILED. 4 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.09s
    === MUTANT R11 sidecar not consumed [inference_admission_processes]
      test descendant_context_waits_behind_its_root_and_a_forged_context_fails_closed ... FAILED
      test result: FAILED. 4 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.09s

All nine review-fix mutants killed by their intended tests.

## Second adversarial review (2026-09-09, after CI green on #1697)

Verified 13 of the first-round fixes CLOSED and 4 PARTIAL (H3, H4a, M5, L5,
all about abnormal exits and layouts), and found 11 new items. Each fix landed
with a failing test first (RED captured as compile/assertion failures in the
broker, process, config and real-process suites).

|Finding|Fix|Proof|
|---|---|---|
|R2-1 medium: any non-orderly exit leaks a live scope until reset|`Actor::closed` releases the scope when the disconnect leaves nothing uncertain (`AdmissionAuthority::release`); unverified work still keeps it; readiness rollback retires the child and removes its sidecar|`closing_without_uncertain_work_retires_the_scope`|
|R2-2 medium: mask missed `directory == ~/.quecto` and symlinked/aliased paths|config refuses a directory at or above the base dir; `create.sh` compares `realpath -m` results and dies on equality|`authority_directory_may_not_be_the_base_dir_itself`; container e2e reran green|
|R2-3 low-medium: a session binding a second capability leaves a stale binding|second `Bind` on a bound session refused|`a_session_cannot_bind_a_second_capability`|
|R2-4 low: raced-cancel completion had no journal retry|routed through `spawn_completion`|existing `cancel_after_grant...`|
|R2-5 low: one warning per 250 ms probe during an outage|log on failure transitions and recovery only|reviewed|
|R2-6 low: sidecar with a valid token left after a non-credential failure|child removes the sidecar on every outcome; parent removes it on readiness rollback|process tests (`!forged.exists()`, `!unreachable.exists()`), BDD updated|
|R2-7 low: clients created authority directories|`AuthorityDirectory::existing` validates without creating|process test asserts the directory is not created|
|R2-8 low: SIGTERM test signalled the shared lib test binary|moved to the real-process suite (`sigterm_stops_the_authority_cleanly`); `run` no longer covered in-lib|lib gate still 92.20%|
|R2-9 low: replayed queued acquire displaced the waiter|replay observes the queued state; the original waiter keeps the grant|`a_replayed_queued_acquire_keeps_the_first_waiter` (raw framed client)|
|R2-10 note: 256 KiB frame cap per connection|accepted; same-UID peers only, documented threat model|—|
|R2-11 note: `process::current()` service locator in the launch path|accepted for P3 (scope registration, not admission state); candidate for P4 injection|—|
