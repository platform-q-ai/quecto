# Issue 1680 implementation evidence

Baseline: `12508c67015c20396b747d4da39a91602cca7958` (`master`).
Initial implementation: `08e5d590` on `codex/issue-1680-swarm`.

## Verification chronology

The initial packaged-helper contract failed with `ModuleNotFoundError: swarm`
before the helper existed. The real SQLite contract then passed. Existing Python
execution regressions were preserved through the rename and config extraction.
The first BDD pass was 26/27: it exposed the changed missing-script diagnostic;
the bootstrap was corrected to preserve the interpreter error contract.

The first workspace pass found the missing tool-description example (3811 passing
harness tests, one failure). The focused regression passed after adding it.
Subsequent full workspace unit/binary verification passed **6200 tests**.

Two later lifecycle regressions have separately observed RED and GREEN results:

- `expired_budget_falls_back_to_termination_when_abort_endpoint_is_unavailable`:
  RED: a real sleeper survived deadline settlement when its recorded socket did
  not exist. GREEN: bounded abort failure falls back to terminating the known
  process, and the child is reaped within the test deadline.
- `test_coordinator_death_leaves_readable_failed_progress`: RED: reading the
  summary raised `invoking member is unknown or death confirmed`. GREEN: the
  summary remains readable, records `failed`, retains progress, and rejects new
  work.

The suite includes real SQLite contention, concurrent admission/claim races,
request retries, dependency rejection, stale claims, symlink aliases, all-or-nothing
file sets, bounded messages/pages, cancellation and budget exhaustion. Rust tests
exercise packaged loading, planted-host-store rejection, startup counts, launch
rollback, process start identity, config migration, and background cancellation.
A fake-provider agent-loop run decomposes work, records/resolves a blocker, submits
revision-specific artifacts, verifies them and completes the run. Some broader
contract assertions were added after their helper implementations; they are
regression coverage, not claimed as individually observed pre-implementation RED.

## Adversarial workflow loop 1

Used the repository's built-in `adversarial-review` workflow from
`src/domain/workflow/engine/templates.rs` (the mirrored fixture is
`tests/fixtures/adversarial-review.json`): scope → inspect → challenge → validate
→ report. Scope was the complete initial implementation against the baseline,
including the issue's container, lifecycle and verification invariants.

Finders covered removed behavior, cross-file lifecycle effects, security,
performance/bounds, architecture/reuse, and test falsifiability. Verification
was a separate pass attempting to refute each candidate. This was sequential
self-review, **not an independent review**.

| Candidate | Verification | Resolution |
|---|---|---|
| Summary grows with the full board | CONFIRMED: scratch copy of `08e5d590` returns 55 tasks where the bounded-summary regression expects 50 | Paginated task/file APIs; summary pages plus complete counts; file-board bound |
| Ready failure retains a reserved slot | CONFIRMED: initial rollback waited for termination without updating membership | Prepared launch retains reservation ownership; confirmed rollback releases it; reaper and next admission reconcile deaths |
| Dead coordinator prevents reading partial progress | CONFIRMED against a scratch copy of `08e5d590`; summary raises on the dead reader identity | Read-only access remains available; confirmed coordinator death fails the run |
| Deadline abort failure can leave coordinator running | CONFIRMED by the real-process RED test | Fall back to known-process termination when abort is not accepted |
| New native tool lacks an example | CONFIRMED by the existing registry contract | Added explicit executable API example |

The fixes were applied after validation. Scratch baseline probes deliberately
failed for the two Python regressions; the current helper suite passes them.

## Adversarial workflow loop 2

Repeated the same five workflow stages over the resulting complete change,
including the first loop's fixes. Retried the failure mechanisms and traced the
composition path: official adapter context → startup admission → native tool →
packaged helpers → launch reservation → ready/rollback/reaper → settlement.

- Removed behavior: resource configuration aliases preserve all old limits;
  execution/output/cancellation regressions retained; one tool name remains.
- Cross-file lifecycle: nested containers rejected; setup members counted;
  startup and local launches share the store; rollback waits before releasing;
  idle/ambiguous members retain capacity; terminal state rejects new work.
- Container gate: adapter identity and a differing PID namespace are required;
  a planted coordination directory does not enable the host tool.
- Security/correctness: compiled helper loading under `-I`; bound sender identity;
  parameterized SQL; revision-specific evidence and coordinator-only acceptance;
  claim/reservation tokens and conservative death reconciliation.
- Bounds/performance: short transactions, bounded contention, capped inboxes and
  request ledger, task/file pagination, no wake hint after read-only execution.
- Falsifiability: real SQLite, real process rollback/termination, and fake-provider
  execution; no live provider rate-limit experiment.

No additional candidate survived this second verification pass. This remains
sequential self-review, not a claim of independent assurance or absence of bugs.
The operational limits and cooperative trust model are documented in `docs/swarm.md`.

## Check commands

```sh
PYTHONDONTWRITEBYTECODE=1 python3 quecto-agentic-harness/tests/swarm_helpers_test.py
cargo test --workspace --lib --bins
QUECTO_TAG=swarm cargo test -p quecto-agentic-harness --features test-support --test bdd
cargo test -p quecto-agentic-harness --test swarm_coordination --test architecture --test contracts
cargo clippy --workspace --all-targets --features quecto-agentic-harness/test-support -- -D warnings -W clippy::cognitive_complexity -W clippy::too_many_arguments -W clippy::too_many_lines
scripts/check-quality.sh
scripts/check-bdd-quality.sh
scripts/check-bdd-tags.sh
cargo fmt --all -- --check
```

Authoritative CI is triggered with `merge-requested`; fixes require pushing and
reapplying that label. Merging requires separate explicit user approval.

## Authoritative CI follow-up

The first CI run passed static quality, dependency policy, coverage, mock LLM E2E
and TUI BDD. Its broader integration/architecture checks found two missed
boundaries: runtime brand names in Rust core, and file I/O in the application
end-to-end test. The core now accepts an adapter-issued `isolated-pid-v1` context
plus distinct PID namespace proof; runtime selection remains in scripts. The
fake-provider test moved intact to the integration-test layer with explicit
composition. Neither architecture guard was weakened.

CI also reproduced a pre-existing REPL test race: invalid configuration can exit
before the test finishes writing stdin. The test harness now accepts only
`BrokenPipe` for that early exit, retaining every status/output assertion and
rejecting other I/O errors. The affected container-runtime (8 tests), production
REPL (6 tests), and fake-provider integration checks passed after these fixes.
