//! The process panic hook (#2192, ADR-0029).
//!
//! Release builds unwind, so a panic inside a tool call can be contained to
//! that call (`AgentLoopImpl::execute_contained`). Everywhere else the
//! harness stays fail-fast, as it was under `panic = "abort"`: this hook
//! aborts the process for any panic that is not inside an open tool-call
//! scope, after reporting it as the default hook does. A panic inside a
//! scope is recorded on that scope (its message and location, which the
//! contained result and the `error` event report), reported in one stderr
//! line, and left to unwind. A second panic on a thread already unwinding a
//! contained one (a destructor that panics during the unwind) cannot be
//! contained: Rust aborts on it, so it is reported and ended like any other
//! fatal panic, with the call's first panic it struck. A panic inside an
//! `extern "C"` function is one such case: its first hook call can unwind
//! and is contained like any other, then Rust stops the unwind at the
//! function's no-unwind boundary with a second panic that cannot unwind,
//! which the hook reports as fatal with the first. A panic that says it
//! cannot unwind, or whose info does not say, is fatal. Every line the hook
//! writes ignores a failed write: a panic in the hook would abort without
//! the report.
//!
//! A fatal panic, once reported, also leaves a crash record and an `error`
//! event in the target the agent armed once it claimed its session, so its
//! parent can say why it died. A contained panic leaves a provisional
//! record of its own call, withdrawn when that call ends, however it ends.
//!
//! Installed first thing by the `quecto` binary. Tests and embeddings that
//! never install it keep Rust's default behaviour.
use std::panic::PanicHookInfo;
use std::sync::Once;

use crate::application::tool_panic_scope::{self, PanicSite, Recording, ToolScope};
use crate::domain::crash_record::{CrashRecord, PanicReport};
use crate::infrastructure::persistence::crash_record;

/// What the hook does with one panic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Inside a tool call: record it on the call's scope and let it unwind
    /// to the call's containment.
    Contain,
    /// Anywhere else: report it and abort the process.
    Abort,
}

/// The one rule: a panic is contained only inside an open tool-call scope,
/// only when it is the first on its thread since it entered the scope, and
/// only when its info says it can unwind — one that cannot ends the process
/// whatever the hook decides. A panic in an `extern "C"` function is not
/// such a panic at first: it can unwind, and is contained, until the
/// function's no-unwind boundary stops it with a second panic (that one
/// cannot unwind and is already a second), which is fatal.
pub fn disposition(
    scope: Option<&ToolScope>,
    already_unwinding: bool,
    can_unwind: bool,
) -> Disposition {
    match (scope, already_unwinding, can_unwind) {
        (Some(_), false, true) => Disposition::Contain,
        (Some(_), true, _) | (Some(_), false, false) | (None, _, _) => Disposition::Abort,
    }
}

/// Whether the panic `info` describes can unwind, read from its `Debug`
/// text: stable Rust keeps `PanicHookInfo::can_unwind` private
/// (rust-lang#92988) but prints it. Only an explicit `can_unwind: true`
/// says it can; a text that does not say (a future `Debug` format) is taken
/// as fatal — the fail-fast answer.
pub fn can_unwind(info: &PanicHookInfo<'_>) -> bool {
    can_unwind_in(&format!("{info:?}"))
}

/// [`can_unwind`] of an info's `Debug` text. The field is read from its
/// last mention: it follows the location, whose file name could spell it.
/// Its value must be exactly `true`, ended by `,`, ` `, `}` or the text.
pub fn can_unwind_in(debug: &str) -> bool {
    let value = debug
        .rsplit_once("can_unwind: ")
        .map(|(_, rest)| rest.split([',', ' ', '}']).next().unwrap_or(rest));
    match value {
        Some("true") => true,
        Some(_) | None => false,
    }
}

/// The one line a contained panic leaves on stderr: the default report is
/// not printed for it, and the call's result and `error` event carry it.
/// It says the panic may still become fatal: the hook cannot tell a panic
/// that will be stopped at an `extern "C"` boundary (or struck by a second
/// panic while it unwinds) from one that reaches the call's containment,
/// and those end with a fatal report after this line.
pub fn contained_report(tool: &str, site: &PanicSite) -> String {
    format!(
        "quecto: tool '{tool}' panicked at {}: {} (contained to its call unless a fatal report follows)",
        site.location.as_deref().unwrap_or("an unknown location"),
        site.message
    )
}

/// Write one line to stderr, ignoring a failed write: the printing macros panic
/// on one (a closed pipe, EIO), and a panic in the hook is an abort.
fn report_line(line: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr().lock(), "{line}");
}

/// What a fatal panic adds to the report the previous hook printed: the
/// tool calls running when it happened, and — for a second panic inside a
/// call — the call's own panic it struck.
pub fn fatal_context(running: &[String], struck: Option<&PanicSite>) -> Option<String> {
    let calls = match running {
        [] => None,
        calls => Some(format!("tool calls running: {}", calls.join(", "))),
    };
    let struck = struck.map(|site| {
        format!(
            "it struck the call's own panic while that unwound: {} at {}",
            site.message,
            site.location.as_deref().unwrap_or("an unknown location")
        )
    });
    match (calls, struck) {
        (None, None) => None,
        (Some(calls), None) => Some(format!("quecto: fatal panic ({calls})")),
        (None, Some(struck)) => Some(format!("quecto: fatal panic ({struck})")),
        (Some(calls), Some(struck)) => Some(format!("quecto: fatal panic ({calls}; {struck})")),
    }
}

/// Where and why a panic happened, from the hook's view of it.
pub fn panic_site(info: &PanicHookInfo<'_>) -> PanicSite {
    PanicSite {
        message: tool_panic_scope::payload_message(info.payload()),
        location: info.location().map(|at| {
            format!(
                "{}:{}:{}",
                workspace_location(at.file()),
                at.line(),
                at.column()
            )
        }),
    }
}

/// A panic's source file as it is recorded (#2192 review): relative to the
/// crate it is in, never an absolute build path. A relative path (what a
/// default build gives) is kept as it is; an absolute one keeps only its
/// crate directory onward — the component before its last `/src/` — or,
/// with no `/src/` in it, its file name.
pub fn workspace_location(file: &str) -> &str {
    match (file.starts_with('/'), file.rfind("/src/")) {
        (false, _) => file,
        (true, Some(src)) => {
            let crate_start = file[..src].rfind('/').map_or(1, |slash| slash + 1);
            &file[crate_start..]
        }
        (true, None) => file.rsplit('/').next().unwrap_or(file),
    }
}

/// Install the hook once per process; later calls do nothing. The
/// previously installed hook (Rust's default, which prints the panic) runs
/// first on a fatal panic, so the panic reaches stderr before anything else
/// is attempted.
pub fn install() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        let end = FATAL_END;
        #[cfg(any(test, feature = "test-support"))]
        let end = test_support::fatal_end(end);
        tool_panic_scope::on_call_end(crash_record::withdraw);
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let scope = tool_panic_scope::current();
            // Only a panic inside a scope can be a second contained one.
            let already_unwinding = match &scope {
                Some(_) => tool_panic_scope::begin_contained_unwind(),
                None => false,
            };
            let disposed = disposition(scope.as_deref(), already_unwinding, can_unwind(info));
            // A scope that closed since it was looked up records nothing: the
            // call's result is already read, so the panic is fatal (#2192
            // review).
            let recorded = match (disposed, &scope) {
                (Disposition::Contain, Some(scope)) => {
                    // On the scope first: the call's answer needs nothing more.
                    let site = panic_site(info);
                    let line = contained_report(scope.tool(), &site);
                    match scope.record_panic(site.clone()) {
                        Recording::Kept => {
                            // Only the kept panic leaves a provisional
                            // record: the one the call answers with.
                            let record = crash(&site, None).in_call(scope.tool()).provisional();
                            leave_provisional(
                                scope,
                                || crash_record::record_provisional(scope.id(), &record),
                                crash_record::withdraw,
                            );
                            Some(line)
                        }
                        Recording::Remembered => Some(line),
                        Recording::Refused => None,
                    }
                }
                (Disposition::Contain, None) | (Disposition::Abort, _) => None,
            };
            match recorded {
                Some(line) => report_line(&line),
                None => {
                    // Reported first: whatever else runs, it is on stderr.
                    previous(info);
                    let struck = scope.as_deref().and_then(ToolScope::recorded_panic);
                    let running = tool_panic_scope::in_flight_tools();
                    if let Some(context) = fatal_context(&running, struck.as_ref()) {
                        report_line(&context);
                    }
                    record_fatal(info, scope.as_deref(), struck.as_ref(), running);
                    end();
                }
            }
        }));
    });
}

/// Leave the provisional record of `scope`'s kept panic: `write` it, note
/// the session it went under for the call's end to withdraw, and — should
/// the call have ended on another thread meanwhile, its end finding nothing
/// noted — `withdraw` it here, so it cannot outlive the call.
pub fn leave_provisional(
    scope: &ToolScope,
    write: impl FnOnce() -> Option<String>,
    withdraw: impl FnOnce(u64, Option<&str>),
) {
    let Some(session) = write() else {
        return;
    };
    scope.note_provisional(session);
    if scope.is_open() {
        return;
    }
    if let Some(session) = scope.take_provisional() {
        withdraw(scope.id(), Some(&session));
    }
}

/// A crash record of `site`, which struck `earlier` if it is a second one.
fn crash(site: &PanicSite, earlier: Option<&PanicSite>) -> CrashRecord {
    let unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default();
    // Secret shapes redacted, as in the call's own result (#2192 review);
    // the report bounds the text.
    let report = |site: &PanicSite| {
        PanicReport::new(
            &crate::domain::redaction::redact_secrets(&site.message),
            site.location.as_deref(),
        )
    };
    CrashRecord::new(report(site), std::process::id(), unix_ms).after(earlier.map(report))
}

/// Leave the crash record and the event log's `error` event of a fatal
/// panic: the calls that were running, and — inside a call, where it is a
/// second panic — that call and the panic it struck.
fn record_fatal(
    info: &PanicHookInfo<'_>,
    scope: Option<&ToolScope>,
    struck: Option<&PanicSite>,
    running: Vec<String>,
) {
    let site = panic_site(info);
    let record = crash(&site, struck).running(running);
    let (record, turn) = match scope {
        Some(scope) => (record.in_call(scope.tool()), scope.turn()),
        None => (record, tool_panic_scope::last_turn()),
    };
    crash_record::record_fatal(&record, tool_panic_scope::FATAL_PANIC_SOURCE, turn);
}

/// How a fatal panic ends the process: the one end every quecto binary uses
/// (`quecto_fail_fast`), an abort as under `panic = "abort"`. Only a
/// test-support build may end it otherwise.
pub const FATAL_END: fn() -> ! = quecto_fail_fast::FATAL_END;

/// Test-support only (#2192): a process a test makes panic fatally on
/// purpose must not die of a core-dumping signal. On this kind of host a
/// SIGABRT reaches the system's core-dump handler (a piped core pattern)
/// whatever the core limit or dumpable flag, and each one is a crash
/// notice for the user. Asked to (`QUECTO_TEST_NO_CORE_DUMPS=1`), the hook
/// ends the process with `_exit(134)` — the status an abort reports to a
/// shell — after the same report. A production build has no such switch.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    /// The switch a test sets on a process it expects to end fatally.
    pub const NO_CORE_DUMPS_ENV: &str = "QUECTO_TEST_NO_CORE_DUMPS";

    /// The exit status of a fatal panic ended without a core dump.
    pub const FATAL_EXIT_CODE: i32 = 134;

    /// `production`, or the core-dump-free end when the switch is set.
    pub fn fatal_end(production: fn() -> !) -> fn() -> ! {
        fatal_end_for(std::env::var(NO_CORE_DUMPS_ENV).ok().as_deref(), production)
    }

    /// The end the switch's value `switch` selects: the core-dump-free end
    /// only for `1`. Pure: it reads no environment.
    pub fn fatal_end_for(switch: Option<&str>, production: fn() -> !) -> fn() -> ! {
        match switch {
            Some("1") => exit_without_core_dump,
            Some(_) | None => production,
        }
    }

    /// Keep a child a test expects to end fatally from dumping core (each
    /// dump is a systemd-coredump record and a desktop "crashed" notice):
    /// its fatal end is `_exit(134)`, never a signal, and its core limit is
    /// zero. The one helper every such test uses.
    pub fn without_core_dumps(command: &mut std::process::Command) -> &mut std::process::Command {
        use std::os::unix::process::CommandExt;
        command.env(NO_CORE_DUMPS_ENV, "1");
        // Runs in the forked child before exec; setrlimit is async-signal-safe.
        // SAFETY: the pre_exec closure only calls setrlimit and checks its result.
        unsafe {
            command.pre_exec(|| {
                let none = libc::rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                };
                match libc::setrlimit(libc::RLIMIT_CORE, &none) {
                    0 => Ok(()),
                    _ => Err(std::io::Error::last_os_error()),
                }
            })
        }
    }

    fn exit_without_core_dump() -> ! {
        // Nothing of this process is used after it: it ends at once.
        // SAFETY: `_exit` runs no destructor or handler and never returns.
        unsafe { libc::_exit(FATAL_EXIT_CODE) }
    }
}

#[cfg(test)]
#[path = "panic_hook_path_tests.rs"]
mod path_tests;
#[cfg(test)]
#[path = "panic_hook_tests.rs"]
mod tests;
