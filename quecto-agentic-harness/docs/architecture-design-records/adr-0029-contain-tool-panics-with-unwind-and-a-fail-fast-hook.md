# ADR-0029: Contain tool panics: unwind in release, abort everywhere else

**Status:** Accepted
**Date:** 2026-09-27
**Scope:** #2192. Replaces the release profile's `panic = "abort"`.

## Context

The release profile set `panic = "abort"`. The setting came from #390, a
binary-size change ("no unwind tables, smaller binary"), justified then by
there being no `catch_unwind` anywhere. It reached today's tree through the
import that finalised ADR-0020 phase 4 (#1353). ADR-0020 itself never
discussed the panic strategy. The choice was a size optimisation, not a
correctness decision.

Under abort, one bug in one tool ended the whole agent. In #2192 the `edit`
tool sliced a string at a non-character boundary. The sub-agent vanished
with nothing in its event log after the `tool_call`, and its parent learned
only that it was "dead". The containment added at the tool-execution
boundary (`catch_unwind` around each call) did nothing in release builds,
because an aborting panic never unwinds.

The fail-fast property of abort is still wanted everywhere else. A panic in
the agent loop, the UDS dispatch, a monitor, a reaper or a teardown path
means the harness's own state can no longer be trusted. Carrying on would
risk exactly the half-updated state that abort rules out. Under unwind,
tokio would also quietly turn a panic in any spawned task into a
`JoinError`, which is worse than abort.

## Decision

1. **Release builds unwind** (`panic = "unwind"` in `[profile.release]`).
2. **Every binary installs a panic hook first thing in `main`.**
   - `quecto` installs `interface::panic_hook`.
   - `quecto-tui`, `quecto-api`, `quecto-mcp` and `quecto-runtime-manager`
     run no tool calls. They install the shared `quecto_fail_fast` hook,
     which reports the panic and aborts, so they behave exactly as under
     abort — before any async runtime starts, so no runtime thread exists
     without it. The TUI's terminal-restoring hook chains to it.
   - An architecture test lists the binaries through `cargo metadata` and
     checks each one's first statement. A process test of
     `quecto_fail_fast` checks the shared hook really ends a process on a
     panic on another thread (its own tests end it with `_exit(134)`, never
     a core-dumping abort).
3. **The harness hook decides by one rule: is the panicking code inside an
   open tool-call scope, and is it the first panic there?**
   - The first panic inside a scope is recorded on the scope (message and
     location) and left to unwind to the call's `catch_unwind`. The call
     answers `internal error in tool '<name>': <message>; the call stopped
     at the panic, and any partial effects it had already made may remain`.
     An `error` event (source `tool_panic`, with tool, message and
     location) goes to the event log, and the turn goes on. The message
     reaches the provider and the log with known secret shapes redacted and
     cut to 1 KiB (on a character boundary, marked "…"). The location is
     relative to its crate (`quecto-agentic-harness/src/…`): an absolute
     build path keeps only its crate directory onward.
   - A second panic on a thread already unwinding a contained one (a
     destructor that panics during the unwind) cannot be contained — Rust
     aborts on it — so it is fatal like any other. The hook tells it apart
     by a per-thread flag it sets when it lets a panic unwind; the flag is
     cleared by the containment that catches the panic, or when a scope is
     next entered or left with no panic in progress — never mid-unwind, so
     a scope a destructor enters and leaves during the unwind does not end
     it.
   - A panic that cannot unwind ends the process whatever the hook decides,
     so the hook treats it as fatal. Stable Rust keeps
     `PanicHookInfo::can_unwind` private (rust-lang#92988) but prints it in
     the info's `Debug` text; the hook reads the field from there (a unit
     test pins the real text). Only an explicit `can_unwind: true` lets a
     panic be contained: a missing or unrecognised field (a future `Debug`
     format) is taken as fatal, the fail-fast answer. It is then reported,
     with its calls, and ended like any other fatal panic — so it gets the
     full report, and the test-support core-dump switch applies to it.
   - A panic inside an `extern "C"` function is **not** fatal from the start.
     Its first hook call says it can unwind, so it is contained, recorded
     and reported like any other. The unwind then reaches the function's
     no-unwind boundary, where Rust raises a second panic that cannot
     unwind. That one is a second panic in the scope, so it is fatal, and
     the fatal report names the first as the panic it struck. A process
     test (`a_panic_in_an_extern_c_function_is_fatal_at_its_boundary_and_names_the_first`)
     pins the sequence.
   - A contained panic is not given the default report; the hook writes one
     line instead: `quecto: tool '<name>' panicked at <location>:
     <message> (contained to its call unless a fatal report follows)`. The
     hook cannot know at that point whether the unwind will reach the
     call's containment (an `extern "C"` boundary or a panicking destructor
     may stop it), so the line says so. Every line the hook writes ignores
     a failed write (`writeln!` on a locked stderr, never `eprintln!`): a
     panic in the hook is an abort with no report.
   - Any other panic: the hook reports it (the previous hook runs first),
     then adds the tool calls that were running and, for a second panic
     inside a call, the call's own panic it struck, and ends the process.
   - The calls running come from a process-wide in-flight registry: a
     call's entry is added when its scope starts and removed when its
     future is dropped. Its mutex is held only for a push or a removal; the
     hook reads it with bounded `try_lock` retries and never waits. A
     call's own panic is kept on its scope with the thread that raised it,
     behind a mutex held only to set, read or forget it; the hook takes it
     with bounded `try_lock` retries too. The first panic of each other
     thread in the call is remembered after it (up to eight threads; a
     ninth pins the last entry so it can no longer be forgotten), so a
     thread that catches its own panic never erases another thread's.
   - A tool call's code that handles a panic itself does so through
     `tool_panic_scope::catch_in_call`. A panic caught there is over: the
     per-thread flag is cleared (so a later panic in the call is contained
     as its first, not taken for a second one) and the panic it recorded on
     the call's scope is forgotten (so a call that handled its panic still
     succeeds). A panic the same thread recorded before the catch, or one
     another thread recorded, is kept. An architecture test (syn, per
     function, not per file) allows a raw `catch_unwind`/`CatchUnwind` only
     in `catch_in_call`, the call's containment (`execute_contained`) and
     one test-only helper. A unit test checks that nothing the hook calls
     prints through a panicking macro or panics (`unwrap`, `expect`,
     `assert!`, …).
   - Known limit: `catch_in_call` cannot catch a panic raised while a
     contained panic is already unwinding on the same thread (in code a
     destructor runs during that unwind). The hook sees a second panic in
     the scope and ends the process before the catch is reached, exactly
     as for an uncaught second panic.
   - Known limit: a third-party library that catches a panic in-thread,
     inside a scope, bypasses `catch_in_call`. The flag and the record it
     leaves stay: the call then fails with the handled panic, and a real
     panic later in the same poll is taken for a second one and is fatal.
     No dependency the tools use does this today.
   - The process ends with `abort()` (`quecto_fail_fast::FATAL_END`, the
     one end every binary uses). A test-support build asked to
     (`QUECTO_TEST_NO_CORE_DUMPS=1`) ends it with `_exit(134)` instead —
     the status an abort reports to a shell — because a SIGABRT reaches a
     piped core-dump handler whatever the core limit, and every test that
     ends a process fatally on purpose sets it. A production build has no
     such switch.
4. **The scope travels with the call's future, not with a thread.**
   - `application::tool_panic_scope::scoped` wraps the call's future. Every
     `poll` sets a thread-local marker for the duration of that poll and
     restores the previous value on exit, including during unwinding.
   - A call that awaits and resumes on another worker thread is therefore
     in scope on every poll, on every thread. Code that runs between its
     polls, on the same thread, is never in scope.
   - The scope covers the whole of `ToolExecutor::execute`: the registry's
     lookup and runtime-policy checks and every `ToolGuard`, not only the
     tool itself. A panic there fails the call closed — the tool does not
     run — and is reported like a panic in the tool. The execution
     admission the loop asks before the call starts is outside the scope.
   - The scope closes when the call's future is dropped. Anything still
     running after that is outside it, so a panic there is fatal. Closing
     clears the scope's open flag, then takes its record lock, so a record
     in progress lands first; a record is taken only while the scope is
     open, under that lock, and the hook treats a refused one as fatal. The
     call's containment reads the record only after the close (asserted),
     so a panic in carried work — the detached `find` owner thread's
     teardown included — either fails the call or ends the process; it is
     never lost.
5. **A call's own work, handed to another task or thread, is carried and
   joined.** `infrastructure::tools::call_work` is the only place that
   carries a scope: `spawn_blocking_in_call`, `spawn_blocking_in_call_on`
   and `spawn_in_call` run the work in the call's scope and return a
   `CarriedJoin`, whose `await` resumes a panic in the call;
   `std_thread_in_call` starts a thread in it (the `find` owner thread).
   That std thread is the one exception to joining: it is detached, so its
   panic never resumes in the call. It is still recorded on the call's
   scope, and the agent loop's backstop fails a call that returned
   normally after a panic was recorded in its scope. A panic in carried
   work can therefore never be read as "no output". Grep, grep-rank, the bash capture and saved
   output, environment commands, the spawn tool's launch reservation and
   the swarm tool's own blocking work are carried (the swarm admission
   check carries too, but runs before any scope, so its panic is fatal).
   Work that outlives its call is not: a swarm background job, reapers,
   cleanup and request accounting. A panic in them is fatal, deliberately.
   The `find` owner thread is carried: a panic in it while its call runs
   is the call's; once a cancelled call is gone it only reaps the child,
   outside any scope.
6. **Poisoned locks do not cascade.** Every lock tool code shares recovers a
   poisoned guard with `into_inner`: the swarm job registry and job state,
   the swarm scope, the owner-exit announcement and the inherited tool
   policy, whose data is replaced by single assignments, and the workflow
   engine, which every reader and writer reaches through
   `domain::workflow::lock_engine` — the `bash` guard, the workflow tool,
   the save and the nudges alike read it as it was left. This matters most
   in `Drop` paths such as the swarm scope, where a second panic during an
   unwind would abort. The `recall` tool's locks cannot be poisoned: nothing
   in their critical sections can panic.
   - This is **audited by hand**, not enforced by a tool. The audit
     (2026-09-27, #2192) covered every `Mutex`/`RwLock` that code a tool call
     runs shares with code outside the call: the locks under
     `infrastructure/tools/` and the workflow engine, including the
     `interface/cli` pending-workflow, snapshot and nudge paths that read it.
     Each is listed above; `recall`'s are the one exception, argued above.
     Locks tool code never takes (the UDS cancel slot, test hooks) were out
     of scope and keep `expect`, since a panic under them is fatal anyway.
     A future tool
     that shares a lock with code outside its call must take it through
     `domain::workflow::lock_engine` (the workflow engine) or recover the
     guard with `unwrap_or_else(PoisonError::into_inner)` (the `recovered`
     pattern the swarm registry uses) — never `unwrap`/`expect`, which turns
     one contained panic into a fatal one elsewhere. Reviewers check this;
     an architecture test may take it over later.

## Why unwinding is safe now

- The only frames a contained panic unwinds through are the call's own. The
  agent loop holds no lock across the call's await.
- Every other panic still aborts, from the hook, before any unwinding
  happens. The rest of the harness therefore keeps the exact semantics it
  had under `panic = "abort"`.

## Consequences

- One tool bug costs one call.
- Release binaries are somewhat larger, because they carry unwind tables.
- Code outside the harness's `main` (tests, embeddings) has no hook, so
  Rust's default behaviour applies there. The `panic_hook` process tests
  re-execute the test binary to prove the installed behaviour. Every test
  that ends a process fatally on purpose goes through one test-support
  helper (`panic_hook::test_support::without_core_dumps`): it sets
  `QUECTO_TEST_NO_CORE_DUMPS=1`, so the hook of a test-support build ends
  the process with `_exit(134)` instead of `abort()`, and it sets the
  process's `RLIMIT_CORE` to zero as well. No core-dumping signal is sent,
  so no core dump is produced.
- Carrying is opt-in, because only the tool knows which work belongs to the
  call; an architecture test keeps every carried task joined.
