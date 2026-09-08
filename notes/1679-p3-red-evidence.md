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
