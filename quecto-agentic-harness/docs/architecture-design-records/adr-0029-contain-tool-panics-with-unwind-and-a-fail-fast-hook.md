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

7. **A fatal panic leaves a record of why the process died.** At startup
   the agent prepares its crash target (`<base>/audit/crash`, a directory
   of its own beside the event logs, and the log's crash line) and arms it once it has claimed its
   session, removing the records an earlier run of the session left and
   never touching another owner's. The target follows the session: a
   switch moves it to the new key, and a session that leaves nothing behind
   disarms it. When the agent's event log was the departing session's own,
   the switch also opens the arriving session's log (same parent, same
   cap): the agent writes its events there from then on, and the crash line
   moves with it, so a fatal `error` event lands in the log of the session
   it happened in. A log of its own key (an agent without a session) stays.
   The hook takes no lock: the target is set once and the session it
   records for, and the crash line, are swapped atomically (each old value
   leaked, never freed). A dispatch test drives `/new` and checks both
   moved.
   - A fatal panic, after it is reported, writes `<digest>.crash` (message,
     location, the calls running, and — for a second panic inside a call —
     that call and the panic it struck) and an `error` event (source
     `panic`, its message redacted of secret shapes and bounded as the
     call's own result is, filed under the turn it happened in: the struck call's, or
     the last call's for a panic outside any; a process test checks the
     real hook) within the log's cap and only while the log is not capped:
     the async writer marks the log capped, in state it shares with the
     crash line, before it writes `log_capped`, so nothing follows that
     record — the crash record still says why. The cap is exact: the
     crash line and the async writer reserve each line's bytes from one
     shared budget (an atomic compare-and-swap, no lock — the hook takes
     none) before writing it, so neither can take room the other has
     already reserved, nor overlook bytes the other wrote. A reservation
     whose write fails stays taken (part of the line may have landed). A panic outside any call
     is attributed to none of the running calls.
   - A contained panic (the one its call keeps) writes its own call's
     provisional record, `<digest>.crash.provisional.<pid>.<scope>`, removed
     by name when the call ends however it ends (contained or cancelled),
     or when the call's code catches the panic itself (`catch_in_call`).
     One still there means the process ended first. A reader prefers the
     fatal record.
   - A record is named by `<digest>`, the SHA-256 of the session key's
     exact bytes in hex, never the key's readable file name: that maps
     `:` to `_` for a legacy-safe key, so `telegram:123` and
     `telegram_123` would share a record, and one session's new run would
     clear the other's. The digest has a fixed length whatever the key's.
     Each record also says the exact key it was written for, and a reader
     takes a record only when it names the session asked about — one
     naming another session, or none, is rejected.
   - A new run of a session also clears the temporary files its writes
     left (a process killed between creating one and renaming it), and
     each start sweeps the crash directory of every session's records and
     temporary files older than 30 days (`STALE_RECORD_AGE`): a session
     never resumed does not keep its records for ever. Both remove only
     names of a record's exact shapes (`<digest>.crash`,
     `<digest>.crash.provisional.<pid>.<scope>`, and `.<either>.<16
     hex>.tmp`); the sweep removes only regular files, judged by their
     own modification time (`fstatat`, no link followed), from a bounded
     number of entries examined, and says when it stopped short.
   - Crash records live apart from the event logs (`audit/crash/`, not
     `audit/`): every session's `.jsonl` log is in `audit/`, and a bounded
     listing of it could miss a record, or have one crowded out by names
     anyone can create. The fatal record is read by name; only provisional
     records are listed.
   - A reader asks for one process's records: the pid it expects, when it
     knows one. The fatal record counts only if it names that pid; a
     provisional record only if it is under that pid's name and names the
     same pid. A record naming another pid — however new — never stands
     in for the expected one. The pid is the record's own JSON, not
     authenticated: within the same-uid trust model anyone who can write
     `audit/crash/` can forge a record naming the expected pid, and it is
     accepted. The check keeps one process's records from being taken for
     another's; it is not a defence against a forger with write access. A
     reader that knows no pid reads the newest of any, and believes none
     of it.
   - Provisional names are read only in their exact form,
     `<digest>.crash.provisional.<pid>.<scope>` in decimal digits, the newest
     scopes first and at most 64: names that merely start like one
     (squatters) are skipped before the bound, and skipping is logged. A
     squatter using well-formed names with the expected pid can still
     crowd the bound; that is the same forgery the pid check does not
     stop.
   - A provisional record is withdrawn from the session it was written
     under — the scope notes that key — not from the one the process
     records for by then (a switch between the two would leave it). The
     call's end always withdraws, noted or not (removing a missing record
     is no error); a hook that notes its record after the call ended on
     another thread sees the scope closed and withdraws it itself, so a
     provisional record never outlives its call.
   - Records are written whole (a temporary file renamed into place)
     through the crash directory's handle (`audit` and `crash` each opened
     without following a link), never through a link, and listed and read
     through that handle too — never by path — from at most a bounded
     number of entries, only from a regular file of bounded size, without
     blocking on a FIFO; every text is bounded again on read. A listing
     that stops short of the directory's end, at its bound or on a read
     error, says so (logged), never silently. Every entry read counts
     toward the bound, names that are not UTF-8 included, and a listing
     that reaches the bound reads once more to tell a directory of exactly
     that many entries from a larger one.
   - Removing an entry asks what it is (`fstatat`, no link followed):
     a directory is removed with `AT_REMOVEDIR`, anything else as a file —
     never guessed from the platform's unlink error (`EISDIR` on Linux,
     `EPERM` on macOS).
   - A directory planted under `<digest>.crash` does not stop the fatal
     record: an empty one is removed (by the write, and by a new run's
     clear); a non-empty one leaves the record under this process's
     fallback name `<digest>.crash.provisional.<pid>.0` (no call's scope is
     0, so no call's end withdraws it), found by a reader of that process
     and still marked not provisional.
   - The temporary file's name carries 64 random bits (`getrandom`; a mix
     of time, pid, a serial and an address where none can be had), and a
     name someone already created is skipped for another, up to eight
     times: a co-tenant cannot stop a record by pre-creating its name.

8. **A parent reads how an ended sub-agent ended and what it did.** The
   exit note, `agent_cmd get_state` and `get_messages` on an ended child
   answer from what it left, through the `EndedChildRecords` port and the
   `InspectEndedChild` use case, all in one wording (`ChildEnd::reason`).
   - A crash record is the child's own word — and any container can write
     one under another child's key — so it is believed only when an
     observed end fits a fatal panic of that process (an abort or the
     test-support exit) **and** the child's pid is known — this harness
     launched it — and is the record's own. A row with no pid (a container
     child, a merged descendant) has no record believed, whatever it says;
     and the reader asks the adapter for that pid's records only, so one
     another process planted never stands in. A record next to a clean
     exit — a stale provisional one, or a forged one — is named and not
     believed; one that cannot be tied to the child's process is named as
     such. Even a believed record's attribution ("during tool call
     'edit'") is labelled "(child-supplied)": the tool name is the child's.
     "Believed" means exactly that match — status and pid — and says so
     (`believedBecause` in `get_state`): the record is not authenticated,
     and within the same-uid trust model anyone who can write
     `audit/crash/` can forge one naming the launched pid (or plant squat
     names that crowd the provisional bound). It is a best-effort account
     of why a child died, never proof; hardening it would need a record
     the child cannot forge (e.g. written by this harness from the exit it
     observed), which is out of scope here.
     With no exit status observed (a container child, a merged
     descendant: no pid, no status) nothing ties a record to the child's
     end: the end is unknown, and the record, its attribution included,
     is only quoted as the child's words.
   - Every registry row carries its origin (`ChildOrigin`): `Launched` is
     set only where a launch registers its row; a row a child reports is
     `Reported`; anything else (fixtures, hand-built rows) is `Unverified`.
     A saved roster's rows never re-enter the live registry, so a child
     that ended before a restart is not found for inspection at all. A
     child's report never merges over a `Launched` row — a child naming a
     sibling as its own descendant cannot overwrite that sibling's pid,
     parent or label.
   - Only a `Launched` child's transcript is read. A reported descendant's
     key is its reporter's word: any agent-id (`secret-plan`) turns into a
     session name, so reading it would let a child make its parent read
     another conversation, and no session records who launched it to
     check against. A reported or unverified row's crash record is read only
     under a uuid of the minted form (lowercase, hyphenated) and never as a
     known process's, so it is quoted, never believed.
   - A reported row's display name may not be a launched row's key or
     label either: it is named by its own key instead. Every parent-facing
     label of a reported row — exit notices, ended-child answers — is
     `reported:<key>`, never the name the child chose, so a reported row
     can never read as a child this harness launched.
   - A child's report may not name a row this harness launched (by key or
     by label), the reporter or any of its ancestors (this harness's own
     id included), nor make a row its own ancestor; the prune after a
     merge visits each row once. A report naming the reporter's parent
     under the reporter once made that walk loop forever under the
     registry lock. A lookup by name (`agent_cmd` on an ended child, the
     TUI's history fallback) prefers a launched row, and a reported row
     never stands in for one.
   - The history fallback for an ended child's messages (`get_messages`
     over the UDS) reads a transcript only for a launched row, and the
     saved roster records a session key only for a launched row. An exit
     note offers the transcript ("stays readable with agent_cmd
     get_messages") only when it can be read: a launched child whose
     session this harness's store holds as a regular, non-empty file —
     asked of the entry, nothing read. The note keeps how the end was
     observed (`(process_exit)`, `(connection_closed)`) whatever else it
     says.
   - A default `get_messages` of an ended child keeps the unread-report
     contract a live child's has (#2226): the persisted window (its newest
     256 messages, each with its ordinal and turn origin) is planned by the
     same default-report rule against the row's delivered watermark, its
     report left pending under a receipt the delivery acknowledges — found
     by the receipt when the child is named by a label no live row
     resolves. A report followed by many messages is still the first
     read's; one delivered is not replayed; a report older than the window
     is owed and the read says it is incomplete.
   - `get_report` of an ended launched child answers its final report from
     the transcript, whole up to the final-report budget (64 KiB, #2114),
     cut beyond it with the same notice; `reportFound: false` when it gave
     none. The newest message of an explicit page starts whole too, so
     `get_messages count:1 before:<ordinal+1>` reads any one message whole
     up to the answer's 64 KiB budget. (`get_message` stays TUI-only.)
   - A page of an ended child's transcript holds at most 256 messages,
     whatever `count` asks: more than one 64 KiB answer carries, so a
     larger count only copied messages to discard them. Up to four
     children's transcripts are kept, least recently read given up first
     and together never more than one transcript's 32 MiB bound, so
     readers of two children do not evict each other.
   - Known gaps, left for follow-ups: the UDS `get_messages` history
     fallback reads a launched child's session through the ordinary
     session store (`SessionStore::load`), not the hardened bounded read
     the `agent_cmd` path uses — giving it one needs a new store port
     method, and the port and the store adapter are at their line
     ceilings; and UDS `get_state` on an ended child answers the routing
     error, not the end the `agent_cmd` tool gives, since the dispatch
     context holds no ended-child inspection.
   - A launch whose uuid a child already reported (a guessed uuid)
     replaces the reported row: this harness's own launch is
     authoritative.
   - Whatever of it reaches the parent is escaped, capped (512 bytes for
     the message) and labelled "(child-supplied, unverified)"; a cut never
     falls inside an escape. So is every other child-chosen text: a
     sub-agent's label in every answer and every notice — exit, error,
     completion, stall (capped at 128 bytes), a child's error in its error
     notice (512 bytes),
     and its last tool and error (256 bytes each, under `lastActivity` with
     their provenance). A merged descendant's key and label must fit the
     agent-id grammar (`[a-zA-Z0-9_-]{1,64}`): a label that does not is
     replaced by the key, and a descendant whose key does not is not
     merged; its last tool and error are capped as a direct child's are.
   - The transcript is read only from a regular file, through no link, at
     most 32 MiB of it — the newest part of a larger one, marked
     `olderOmitted`, held to the same append order as a full read — and
     paged by durable ordinal (by position when it has none) within a hard
     64 KiB answer. An unchanged transcript (same device, inode, length,
     modification and status-change time) is shared, not read again; a
     changed one is read afresh.
   - What cannot be read is said, with the reason (a container child whose
     store is not shared with its parent keeps its transcript there).

## Why unwinding is safe now

- The only frames a contained panic unwinds through are the call's own. The
  agent loop holds no lock across the call's await.
- Every other panic still aborts, from the hook, before any unwinding
  happens. The rest of the harness therefore keeps the exact semantics it
  had under `panic = "abort"`.

## Consequences

- One tool bug costs one call, and an agent that does die says why.
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
