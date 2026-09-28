use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};

const CHILD: &str = "QUECTO_FAIL_FAST_CHILD";

/// The child side: a panic on a spawned thread, which would otherwise end
/// only that thread, must end the process.
#[test]
fn child_entry() {
    if std::env::var(CHILD).is_err() {
        return;
    }
    // Ended by an exit, never a core-dumping signal: a crash notice per run
    // is not what a test may cost (the production end is checked below).
    super::end_on_panic(|| std::process::exit(134));
    let _ = std::thread::spawn(|| panic!("a panic on another thread")).join();
    println!("SURVIVED");
}

/// The test-only end of [`super::abort_on_panic`]: the status an abort
/// reports to a shell, without the core-dumping signal.
pub(super) fn exit_without_core_dump() -> ! {
    std::process::exit(134)
}

/// Child of [`the_shared_hook_itself_ends_the_process`]: the real
/// `abort_on_panic`, with its test-only end.
#[test]
fn abort_child_entry() {
    if std::env::var(ABORT_CHILD).is_err() {
        return;
    }
    super::abort_on_panic();
    let _ = std::thread::spawn(|| panic!("a panic under the shared hook")).join();
    println!("SURVIVED");
}

const ABORT_CHILD: &str = "QUECTO_FAIL_FAST_ABORT_CHILD";

/// #2192 review (PRRT_kwDORUxnPM6mf9Bh): `abort_on_panic` — the hook every
/// non-harness binary installs — really ends the process on a panic on any
/// thread (a no-op hook would let the child print SURVIVED and exit 0).
#[test]
fn the_shared_hook_itself_ends_the_process() {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "tests::abort_child_entry", "--nocapture"])
        .env(ABORT_CHILD, "1")
        .env("QUECTO_TEST_NO_CORE_DUMPS", "1")
        .env("RUST_BACKTRACE", "0")
        .stdin(Stdio::null());
    // SAFETY: the pre_exec closure only calls setrlimit and reports failure.
    unsafe {
        command.pre_exec(no_core_limit);
    }
    let output = command.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        (output.status.code(), output.status.signal()),
        (Some(134), None),
        "{stdout}"
    );
    assert!(!stdout.contains("SURVIVED"), "{stdout}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("a panic under the shared hook"));
}

#[test]
fn a_panic_on_any_thread_ends_the_process() {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "tests::child_entry", "--nocapture"])
        .env(CHILD, "1")
        .env("RUST_BACKTRACE", "0")
        .stdin(Stdio::null());
    // Runs in the forked child before exec; setrlimit is async-signal-safe.
    // SAFETY: the pre_exec closure only calls setrlimit and reports failure.
    unsafe {
        command.pre_exec(no_core_limit);
    }
    let output = command.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        (output.status.code(), output.status.signal()),
        (Some(134), None),
        "{stdout}"
    );
    assert!(!stdout.contains("SURVIVED"), "{stdout}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("a panic on another thread"));
}

// SAFETY: this declares libc's `setrlimit(int, const struct rlimit *)` with
// a matching ABI: `RLIMIT_CORE` is an `int` and on the 64-bit Linux targets
// this crate is tested on `struct rlimit` is two `rlim_t` (u64) words.
unsafe extern "C" {
    fn setrlimit(resource: i32, limit: *const [u64; 2]) -> i32;
}

/// A zero core limit too, set before exec.
fn no_core_limit() -> std::io::Result<()> {
    const RLIMIT_CORE: i32 = 4;
    // SAFETY: setrlimit reads only the two-word limit it is given.
    match unsafe { setrlimit(RLIMIT_CORE, &[0, 0]) } {
        0 => Ok(()),
        _ => Err(std::io::Error::last_os_error()),
    }
}

/// Why an address comparison: the production end cannot be run by a test
/// (an abort is a core-dumping signal), and its type `fn() -> !` is shared
/// by every diverging function, so no type-level check can tell `abort`
/// apart. Rust does not promise one address per function in general, but
/// the two ways that goes wrong do not apply here: `abort` is a single
/// non-generic, non-inlined function of `std`, so every reference to it
/// resolves to the one exported symbol (no per-crate copy); and no other
/// function with an identical body exists for the linker to merge it with.
#[test]
fn the_production_end_is_an_abort() {
    assert_eq!(
        super::FATAL_END as usize,
        std::process::abort as fn() -> ! as usize
    );
}
