//! Test-support only (#2192): a process a test ends fatally on purpose (a
//! fatal panic, or a crash it stages) must not die of a core-dumping
//! signal. Where the core pattern pipes to a handler such as
//! systemd-coredump, each SIGABRT is a dump record and a crash notice for
//! the user. Run through [`without_core_dumps`], such a process ends with
//! `_exit(134)`, the status an abort reports to a shell, through
//! [`fatal_end`]. A production build has no such switch.
//!
//! It lives in a layer-neutral test-support module, so a test in any layer
//! uses it without depending on the interface layer.

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
