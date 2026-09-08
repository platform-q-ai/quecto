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

## Review remediation (2026-09-08)

The external review of `da8b355cf7ba67585f85e94656665167ebfce636` identified
three correctness defects and four architectural boundary problems. All seven
were valid. The previous green suite and sequential self-review missed these
cases; they were not evidence of complete lifecycle safety.

### Reproduction and fixes

- **3956977732 — final revision revalidation.** A new real-SQLite regression
  completed A at R1, then its dependent B at R2. Completion at R2 rejected A and
  the required revalidation operation was absent (RED). Coordinator-only
  `revalidate_task` now requires fresh artifact evidence at the requested
  revision and audits the old and new evidence. Worker revalidation and stale
  evidence are rejected. The two-task test and a BDD scenario exercise success.
- **3956977737 — surviving execution scope.** The original reconciliation test
  observed an active writer but zero retained file reservations (RED). The
  strengthened regression starts a real parent and independently grouped writer,
  kills/reaps only the parent, then verifies reconciliation retains the file and
  membership while failing the run. The current adapter cannot prove an orphaned
  execution scope empty: it does not grant replacement claims or in-place crash
  recovery. The environment must be stopped and discarded. Unlaunched
  reservations remain safely releasable; post-launch rollback is conservative.
- **3956977742 — coordinator detached execution.** The initial regression
  observed a late write after `stop('failed', ...)` (RED). Settlement now cancels
  the local execution registry before turn abort, including a coordinator whose
  endpoint accepts abort. The strengthened test gates a real writer until after
  failed settlement and uses an accepting UDS endpoint. Member watchers also
  observe remote terminal outcomes. Registry closure plus serialized
  spawn/identity publication prevents queued jobs escaping cancellation.
- **3957004456 — completion boundary.** Pure Python domain decisions operate on
  criteria/evidence/task values. Application completion and revalidation use an
  injected atomic repository, with in-memory tests that never open SQLite or
  launch a child interpreter. The SQLite adapter supplies the domain snapshot
  and persists the decision under the same transaction.
- **3957004468 — authorization/admission boundary.** Authorization, eligibility,
  deadline and admission decisions live in the pure domain. Application owns
  the clock and operation sequencing; expiry commits independently of a rejected
  mutation. The SQLite adapter retains immediate transactions. Existing real
  contention/admission/claim tests remain; pure fake-clock and repository tests
  add isolated policy coverage. Remaining SQL-facing task/workbench operations
  stay adapters; this is incremental extraction, not duplicate Rust policy.
- **3957004475 — typed coordination port.** Production callers use typed
  snapshots, identities, outcomes and operations. Wire method names, positional
  arguments and JSON decoding stay inside the Python adapter. CLI composition
  registers endpoints through the port. Invalid lifecycle wire data fails
  explicitly instead of silently skipping members. Contract tests cover the
  real packaged adapter and all new public ports.
- **3957004481 — settlement boundary.** Application owns reconciliation,
  settlement ordering, coordinator exceptions, fallback and deadline decisions.
  Process/coordination/clock ports own effects. The interface composition root
  injects `SwarmLifecycle` into tool, spawn and reaper adapters; infrastructure
  neither constructs nor directly imports the application service. Fake-port
  tests require no sockets, process control or persistence. Existing Linux/UDS
  integration tests cover the adapters.

### Additional adversarial workflow loops

Both loops followed the built-in `adversarial-review` fixture's scope, inspect,
challenge, validate and report stages. Scope was the remediation diff against
`da8b355c`, including new files, the seven review claims and existing containment,
transaction, ownership and cancellation invariants. These were sequential
self-review loops, not independent reviews or a claim of perfection.

**Loop 1:** Narrow checks covered cross-language dependency direction, admission
atomicity, cancellation interleavings and process ownership. The queued-background
launch race was confirmed: cancelling current registry entries alone did not
close admission to queued/new jobs. Registry closure and a spawn/cancel critical
section fix it; a current-thread queued-launch regression checks no writer starts
and later registry admission fails. A supervisor must also retain its known
deadline when a transient coordination read fails; the watcher now cancels local
jobs, retries observation and applies the last known budget through the injected
clock policy. Architectural checks caught direct application references from
infrastructure; explicit composition-root injection replaced them, without
weakening the guard or adding an alias to evade it.

**Loop 2:** Rechecked the complete composition path, typed wire validation,
coordinator abort acceptance/failure, unlaunched versus launched rollback,
revision-specific revalidation, atomic admission and expiry, and the new tests'
ability to falsify the claims. Corrected remaining documentation that equated
harness death with execution-scope death. No additional correctness candidate
survived the final source verification. Live container/provider behavior remains
outside this local validation; real SQLite, local orphan-process and UDS tests
cover the reproduced mechanisms.

The PR remains subject to authoritative CI and explicit user approval before
merge. No auto-merge is enabled.

Local remediation validation passed: 6,230 workspace library/binary tests;
46 architecture tests; 104 port contracts; the fake-provider agent-loop test;
19 real-SQLite Python cases and five pure Python policy/use-case cases;
28 swarm BDD scenarios (120 steps); and 10 architecture BDD scenarios (17 steps).
Strict workspace/all-target Clippy and repository quality/status-tag gates passed.
The BDD quality gate retained its existing warnings without hard failures.

## Local product-report remediation

Scope: changes against `94f63c3d508eae115edd58bd4e7132481db36c42`, prompted by
`/tmp/swarm-product-report/REPORT.md`. The report's checkout revision did not
establish its running binary's build identity; its four-agent run is user evidence,
not a run repeated by this remediation. Unrelated local notes and Python caches
were preserved.

Regression tests first failed for nested `.quecto` artifact roots, absent process
limit diagnostics, terminal notification delivery, missing typed acceptance
errors and missing embedded documentation. Green implementations now resolve
artifacts against the explicit workspace, retain the default RLIMIT_NPROC=1 with
Bash routing guidance, document worker evidence proposals separately from
coordinator acceptance, and expose a packaged `docs` manual. Normal summary and
artifact export remain usable after successful completion.

### Product-remediation adversarial-review loops

Two sequential self-review loops followed the built-in `adversarial-review`
fixture's scope, inspect, challenge, validate and report stages. They were not
independent reviews. Both covered the product-report acceptance criteria,
notification authority/transactions, artifact namespace, execution policy and
Clean Architecture dependency direction; each inspection was read-only, with
accepted fixes made between review stages/cycles.

**Loop 1:** Traced notification producers, consumers and terminal settlement.
Confirmed that global audit changes could cause an observer's read to rebroadcast
another member's mutation, acknowledgments could wake the pool, and direct
notification delivery lacked a terminal guard. This supports a concrete feedback
mechanism; it does not prove the exact provenance of every prompt in the user's
report. Target selection now lives in pure domain policy, atomic cursor claiming
in the application use case and SQL/UDS effects in adapters. Real SQLite tests
refute duplicate delivery after reads/acks and concurrent claims; the real UDS
regression rejects terminal wake delivery. The nested-workspace regression
confirmed the first-`.quecto` path heuristic was wrong and now verifies status,
output and synchronous spill references resolve to actual files.

**Loop 2:** Challenged concurrent notification claims, restart persistence,
terminal transitions after hint selection, failed delivery, policy bypass and
misleading documentation. Concurrent clients and a reopened client consume a
hint only once. A delivery can race completion, so the prompt explicitly starts
with summary inspection and permits terminal reporting/export without inbox
execution. Hints remain best-effort: the cursor commits before delivery and
failed hints are not retried forever; durable inbox/tasks remain authoritative.
The default process limit is unchanged, and a real non-root child fails to spawn
while the separately configured Bash tool executes a command. Typed acceptance,
worker proposal wording and embedded manual checks pass. No additional finding
survived these bounded checks; this is not a claim of absence of bugs.

Validation passed: 6,233 workspace library/binary tests; 46 architecture tests;
104 port contracts; fake-provider agent-loop test; 23 real-SQLite Python cases;
six pure policy/use-case cases; five product contract tests; 29 swarm BDD scenarios
(124 steps); strict workspace/all-target Clippy; formatting, quality and BDD tag
gates. The BDD quality gate retained existing warnings without hard failures.

The five compiled product contract tests also passed in a fresh non-root,
network-disabled container with read-only root and temporary scratch storage.
Three further compiled unit tests passed there for coalesced UDS hints, terminal
notification suppression and embedded docs. The container mounted only the test
binary, not the checkout or provider credentials. The PR records the final source
commit, binary SHA-256 digests and image ID for the repeat after commit. This is
container regression validation, not a new live-provider four-agent walkthrough.
