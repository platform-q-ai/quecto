//! `quecto-test-fixture`: the processes the harness's tests stand up around
//! it, in Rust (#2283): the harness has no Python dependency, tests
//! included. Built only with the `test-support` feature; the tests reach it
//! as `env!("CARGO_BIN_EXE_quecto-test-fixture")`.
//!
//! - `processes track|live|clean <dir> [env] [pid]`: identity-scoped
//!   ownership of script-managed fixture processes (`processes.rs`).
//! - `uds-bridge [--close-on-stdin-eof] [--slow-accept-marker <path>]
//!   <socket>`: a stdio<->Unix-socket bridge, a `socket_proxy` argv
//!   (`uds.rs`).
//! - `uds-listen <socket> [--unlink] [--decoy-log <file>] [--accepts <n>]
//!   [--linger <secs>] [--hold] [--idle-exit <secs>]`: a stand-in child's
//!   listener (`uds.rs`).
//! - `admission-peer direct|proxy|nested`: the inference-admission
//!   transport experiment's peer (`admission.rs`).
//!
//! Any failure exits non-zero with its reason on stderr. The process
//! fixture reads `/proc` and signals through pidfds, so it is Linux's
//! alone; elsewhere it refuses (the tests that use it run on Linux).

mod admission;
#[cfg(target_os = "linux")]
mod processes;
mod uds;

#[cfg(not(target_os = "linux"))]
mod processes {
    pub fn run(_args: &[String]) -> Result<(), String> {
        Err("the process fixture is Linux-only".to_owned())
    }
}

fn main() {
    // A fixture that panics fails at once, as every workspace binary does.
    quecto_fail_fast::abort_on_panic();
    // Arguments that are not UTF-8 are refused, never a panic.
    let args: Result<Vec<String>, _> = std::env::args_os()
        .skip(1)
        .map(std::ffi::OsString::into_string)
        .collect();
    let outcome = match args.as_deref() {
        Err(_) => Err("an argument is not UTF-8".to_owned()),
        Ok(args) => dispatch(args),
    };
    if let Err(reason) = outcome {
        eprintln!("quecto-test-fixture: {reason}");
        std::process::exit(1);
    }
}

fn dispatch(args: &[String]) -> Result<(), String> {
    match args.split_first() {
        Some((command, rest)) => match command.as_str() {
            "processes" => processes::run(rest),
            "uds-bridge" => uds::bridge(rest),
            "uds-listen" => uds::listen(rest),
            "admission-peer" => admission::run(rest),
            other => Err(format!("unknown fixture {other}")),
        },
        None => Err("usage: quecto-test-fixture <fixture> [args...]".to_owned()),
    }
}
