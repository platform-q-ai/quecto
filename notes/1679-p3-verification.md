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
|Architecture/quality|architecture 46, contracts 273 (+18 new), quality gate PASS, strict clippy (all targets + test-support) clean, harness lib 4113+6|

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
