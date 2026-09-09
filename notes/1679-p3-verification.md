# #1679 P3 verification / AC evidence

Base 6a383f30 (origin/master after #1688), branch `issue-1679-p3`. No paid
provider traffic: every proof uses a loopback fake HTTP provider or in-process
authority. RED, mutation and container evidence: `notes/1679-p3-red-evidence.md`.

|AC / P3 checklist item|Objective verification|
|---|---|
|AC1 shared bound, real processes|`inference_admission_processes`: authority subprocess + three independent `quecto agent` roots at C=2 -> fake provider peak<=2, total 3, in-flight 0 after; control without `admission` -> peak 3 (oracle sensitivity); descendant sidecar (`--admission-context`) queued behind the root at C=1 (active 1/queued 1), granted after release|
|AC1 uncertain retains capacity; reset new epoch|contracts `admission_recovery`: abandon -> uncertain counted active, `Quarantined`; verified completion clears; `reset` -> epoch+1, old scope `StaleEpoch`, cooldown carried; restore -> orphans counted active+uncertain until reset|
|AC4 attempt lifetime unchanged|P2 suites still green (`inference_admission_attempts` 65, `_runtime` 17, `_transport` 2, `_characterization`, `_retry`, `_feedback_transport`, `_runtime_oauth`); remote permit releases only through `finish` -> `complete` (journal-before-ack), never on drop|
|AC6 bounded queue/deadline, cancel/grant, disconnect, SIGKILL, restart|`inference_admission_broker`: queued acquire dropped -> cancelled at authority within 1 s with 60 s deadline (mutant N2 killed); connection loss -> uncertain 1 -> other root's acquire fails explicitly at the queue deadline -> same capability reconnects and completes -> grants resume; authority shutdown with outstanding permit -> restart keeps epoch, orphan uncertain 1, reset -> epoch+1 and old capability `Unauthorized`; unwritable directory -> acquire denied, `journal_healthy=false`, recovers when writable. Processes: SIGKILL of a root mid-request -> uncertain 1 -> next root fails explicitly -> `admission-broker reset` epoch 2; SIGKILL of the authority -> uds root survives, restart keeps epoch, `status` JSON shows uncertain/active 1|
|AC6 journal-before-grant|contracts `admission_journal`: ledger persisted with outstanding before the grant is returned; journal failure withdraws the undelivered grant and blocks new grants until a durable write; completion is not acknowledged without a durable release; reset is refused when not journalable. File journal: temp+fsync+rename+dir fsync, mode 0600, corrupt file fails closed (never empty restart)|
|AC7 readiness negotiates admission; unsupported/mixed fails visibly|Processes: forged sidecar -> one-shot child exits non-zero naming admission with zero provider attempts; `--mode uds` forged child exits before its control socket ever accepts (mutant P3c). Protocol: unsupported version and unbound operations refused; admin refused on the client socket; `hello required first`|
|AC7 container capability|Unit (`spawn_container_admission_tests`): enabled parents pass `QUECTO_ADMISSION_DIR`, require `shared-directory-v1` on create and exec, disabled parents unchanged. Real podman run (`inference_admission_container`, gated): adapter mounts the client dir, reports the capability, child binds before socket readiness, queues behind root, granted after release; without the dir no capability is reported|
|AC3 restart-only policy|`config_admission_tests`: absent section disabled; validated proposal/directory; invalid reserve, unknown alias, unknown key rejected at load. `catalogue_runtime` composes through the P2 restart-only factory whenever a binding is installed (mutant P2 killed)|
|Authentication/lineage|contracts `admission_journal`: distinct root secrets, forged tokens `Unauthorized`, children need the parent capability and inherit its root, retire revokes; `admission_secret_source`: 64-hex OS randomness, unique|
|BDD|`@inference-admission` lane: 9 scenarios / 49 steps GREEN incl. 3 new authority scenarios over a real in-process authority (shared slot, quarantine/reconcile, reset)|
|Review fixes (H1–L6)|`inference_admission_broker` 16 scenarios incl. interleaved traffic, raced cancel completes as failed, owner-token roots, missing-ledger refusal, supersede on rebind, outage hold with ledger error, framing deadline; contracts: no-poison unknown completion, live-scope limit, parent-only child retirement, journal probe; processes: `live_scopes` 0 after clean exits, sidecar consumed; container e2e: authority root masked, only `client/` visible, failed create leaks nothing|
|Architecture/quality|architecture 46, contracts 277 (+22 new), quality gate PASS, strict clippy (all targets + test-support) clean, harness lib 4116+|

Commands (from repo root, `quecto-wt-1679`):

```sh
cargo test -p quecto-agentic-harness --test contracts admission_
cargo test -p quecto-agentic-harness --test inference_admission_broker
cargo test -p quecto-agentic-harness --test inference_admission_processes
QUECTO_ADMISSION_CONTAINER_E2E=1 cargo test -p quecto-agentic-harness --test inference_admission_container -- --nocapture
QUECTO_TAG=inference-admission cargo test -p quecto-agentic-harness --features test-support --test bdd
cargo test -p quecto-agentic-harness --test architecture --test contracts
scripts/pre-push.sh
```

Not claimed: P4 observation/TUI projection, fairness end-to-end matrix, burst
comparison and rollout runbooks; multi-host or hostile same-UID enforcement;
remote cancellation on disconnect. Normal runtime stays disabled without an
`admission` section. Final gate log lines are appended below after the last run.

## Final gate run (2026-09-08, after review fixes)

```
== docs invariants
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
== lib
test result: ok. 4118 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 21.77s
== tagged BDD
9 scenarios (9 passed)
49 steps (49 passed)
== admission integration
inference_admission_broker: test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 15.02s 
inference_admission_processes: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.09s 
inference_admission_runtime: test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s 
inference_admission_attempts: test result: ok. 65 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.03s 
inference_admission_transport: test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.45s 
uds_termination: test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s 
1455 scenarios (1455 passed)
7063 steps (7063 passed)
pre-push (quality, BDD quality/tags, fmt, strict clippy, architecture 46, contracts 277): PASS after the redundant-guard lint fix
container e2e (QUECTO_ADMISSION_CONTAINER_E2E=1, podman): 2 passed
```

## CI follow-up (2026-09-08, PR #1697 first run)

- Non-Real BDD: `Two independent roots share one capacity slot` flaked on the
  runner because the scenario's 300 ms queue deadline could expire before the
  poll observed the queued request; the capacity-one scenario now uses a 60 s
  deadline (the quarantine scenarios keep 300 ms for their bounded refusal).
- Coverage: the lib gate (`--fail-under-functions 92`) reported 86.33% because
  the P3 adapters were proven only by integration tests. The contract suites
  and the broker suite are now folded into the library gate (`lib.rs`
  `#[path]` includes, the existing pattern), plus unit tests for the process
  binding (`negotiate` split from the global `install` so tests never leak an
  installed authority into sibling tests — the first attempt did, failing 63
  unrelated lib tests), the broker command (`run_until` split from signal
  wiring), config defaults, protocol round-trips and error surfaces. Local
  `cargo llvm-cov --lib ... --fail-under-functions 92`: 92.08% (363/4581
  missed), 4180 lib tests green.
- Second CI run: Non-Real BDD scenarios all passed but that job's own
  coverage gate (`run-bdd-shards.sh --coverage-threshold 72`, functions
  reached by BDD scenarios) reported 71.29% against the 72.23% baseline of the
  last merged PR; the lib gate reported 91.97%. Added
  `inference_admission_operations.feature` (6 scenarios: process binding
  negotiation/feedback/shutdown, descendant sidecars and forgeries, broker CLI
  status/reset/misuse, corrupt/unsupported/missing ledger, owner-token roots
  and child retirement, raced cancel) driving the public API; tagged lane now
  15 scenarios / 81 steps.
- Local gates after the additions: `run-bdd-shards.sh --suite non-real-bdd
  --coverage --coverage-threshold 72` -> 73.22% (1205/4500 missed), all
  scenarios green; `cargo llvm-cov --lib ... --fail-under-functions 92` ->
  92.25% (355/4583 missed), 4182 lib tests. Extra lib-gate margin came from
  `install_in` (injectable slot), `admission_candidate`, a SIGTERM-driven test
  of the broker `run` path and named start-time refusals.
- Second adversarial review: 11 new findings fixed (see red-evidence); after
  the fixes lib admission 297, real-process 6 (incl. SIGTERM), container e2e 2,
  tagged BDD 15/81, pre-push gate green, lib coverage 92.20% (358/4588).
