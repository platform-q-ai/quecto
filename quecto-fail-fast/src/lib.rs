//! Fail fast on any panic (#2192, ADR-0029).
//!
//! Release builds unwind so the agent harness can contain a panicking tool
//! call to that call. A binary that runs no tool calls (the TUI, the API
//! gateway, the MCP extension, the runtime manager) keeps the behaviour it
//! had under `panic = "abort"`: every panic is reported, then the process
//! aborts — a panic in a spawned task never becomes a silent `JoinError`.

/// How a panic ends the process: an abort, as under `panic = "abort"`.
pub const FATAL_END: fn() -> ! = std::process::abort;

/// Install the abort-on-panic hook, chained after the hook already set (by
/// default Rust's, which prints the panic). Called first thing in `main`.
pub fn abort_on_panic() {
    end_on_panic(fatal_end());
}

/// The end [`abort_on_panic`] installs: [`FATAL_END`]. Only this crate's own
/// tests may, when asked (`QUECTO_TEST_NO_CORE_DUMPS=1`), end with
/// `_exit(134)` instead, so a test can watch the real hook end a process
/// without a core-dumping abort.
fn fatal_end() -> fn() -> ! {
    #[cfg(test)]
    if std::env::var("QUECTO_TEST_NO_CORE_DUMPS").as_deref() == Ok("1") {
        return tests::exit_without_core_dump;
    }
    FATAL_END
}

/// Install a hook that reports a panic, then ends the process with `end`.
/// Production passes [`FATAL_END`]; a test that makes a process panic on
/// purpose passes an end that is not a core-dumping signal.
pub fn end_on_panic(end: fn() -> !) {
    let report = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        report(info);
        end();
    }));
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
