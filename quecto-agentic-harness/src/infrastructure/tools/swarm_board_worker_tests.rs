use serde_json::json;

use super::Registry;

fn call(
    checkout: &std::path::Path,
    member: &str,
    bootstrap: &str,
    method: &str,
    args: serde_json::Value,
) -> Result<serde_json::Value, crate::domain::error::DomainError> {
    // Every test owns a registry: the process-wide one would let unrelated
    // swarm tests evict these workers between two asserted calls.
    thread_local! {
        static REGISTRY: Registry = Registry::default();
    }
    REGISTRY.with(|r| r.call(checkout, member, bootstrap, method, args))
}

/// A stand-in board: `echo` returns its arguments, `boom` raises, `quit`
/// ends the interpreter mid-call (an exit with nothing on stderr).
const STUB: &str = r#"import sys, json
class _Board:
    calls = 0
    def echo(self, *args):
        _Board.calls += 1
        return {'args': list(args), 'calls': _Board.calls}
    def boom(self, text):
        raise RuntimeError(text)
    def quit(self):
        sys.exit(0)
class _Swarm:
    board = _Board()
swarm = _Swarm()
"#;

fn checkout(label: &str) -> tempfile::TempDir {
    tempfile::Builder::new().prefix(label).tempdir().unwrap()
}

#[test]
fn calls_reuse_one_interpreter_per_checkout_and_member() {
    let dir = checkout("reuse");
    let first = call(dir.path(), "m", STUB, "echo", json!([1, "a"])).unwrap();
    let second = call(dir.path(), "m", STUB, "echo", json!([])).unwrap();
    assert_eq!(first["args"], json!([1, "a"]));
    // The counter lives in the interpreter: a second process would say 1.
    assert_eq!(second["calls"], 2, "{second}");
    let other = call(dir.path(), "other", STUB, "echo", json!([])).unwrap();
    assert_eq!(other["calls"], 1, "another member is another interpreter");
}

#[test]
fn a_board_exception_is_the_swarm_error_text() {
    let dir = checkout("boom");
    let err = call(dir.path(), "m", STUB, "boom", json!(["nope"])).unwrap_err();
    assert_eq!(err.to_string(), "tool error: swarm: \"nope\"");
    // The interpreter survives its own exception.
    assert_eq!(
        call(dir.path(), "m", STUB, "echo", json!([])).unwrap()["calls"],
        1
    );
}

#[test]
fn an_interpreter_that_exits_mid_call_is_an_error_and_the_next_call_gets_a_fresh_one() {
    let dir = checkout("quit");
    call(dir.path(), "m", STUB, "echo", json!([])).unwrap();
    // `quit` exits before answering. The call is not retried — the board
    // method may already have run — so it fails with the (empty) stderr.
    let err = call(dir.path(), "m", STUB, "quit", json!([])).unwrap_err();
    assert_eq!(err.to_string(), "tool error: swarm coordination failed: ");
    assert_eq!(
        call(dir.path(), "m", STUB, "echo", json!([])).unwrap()["calls"],
        1
    );
}

#[test]
fn a_bootstrap_failure_reports_the_interpreter_stderr() {
    let dir = checkout("bootstrap");
    let err = call(
        dir.path(),
        "m",
        "raise SystemExit('no board here')",
        "echo",
        json!([]),
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .starts_with("tool error: swarm coordination failed: no board here"),
        "{err}"
    );
}

#[test]
fn the_registry_is_bounded_and_evicts_the_least_recently_used() {
    let registry = Registry::default();
    let dirs: Vec<_> = (0..super::MAX_WORKERS + 1)
        .map(|i| checkout(&format!("lru{i}")))
        .collect();
    for dir in &dirs {
        assert_eq!(
            registry
                .call(dir.path(), "m", STUB, "echo", json!([]))
                .unwrap()["calls"],
            1
        );
    }
    assert_eq!(registry.resident(), super::MAX_WORKERS);
    // The first checkout was evicted when the ninth arrived: its counter
    // restarts. The second is still resident and keeps counting.
    assert_eq!(
        registry
            .call(dirs[0].path(), "m", STUB, "echo", json!([]))
            .unwrap()["calls"],
        1
    );
    assert_eq!(
        registry
            .call(dirs[2].path(), "m", STUB, "echo", json!([]))
            .unwrap()["calls"],
        2
    );
}

/// The interpreter prelude binds the worker's life to its parent: when the
/// parent is killed outright, the worker is gone at once — before anyone can
/// look for strays — without the harness signalling it. Modelled with a
/// python parent that spawns a child under the same prelude and is then
/// SIGKILLed; the child deliberately ignores stdin so only the prelude can
/// end it (the real worker also exits on stdin EOF, which this test must not
/// mistake for the prelude working).
#[test]
fn a_worker_dies_with_its_parent_without_being_signalled() {
    use std::io::BufRead;
    // The child announces itself once the prelude has run (the prelude is
    // the first thing the interpreter executes, as in the real worker);
    // killing the parent before that would test nothing.
    let parent_source = format!(
        "import subprocess, sys, time\n\
         child = subprocess.Popen([sys.executable, '-I', '-c', {prelude:?} + 'import time, sys\\nprint(\\'ready\\', flush=True)\\nwhile True: time.sleep(1)\\n'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)\n\
         print(child.pid, flush=True)\n\
         print(child.stdout.readline().decode().strip(), flush=True)\n\
         time.sleep(30)\n",
        prelude = super::PDEATHSIG
    );
    let mut parent = std::process::Command::new("python3")
        .args(["-c", &parent_source])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = std::io::BufReader::new(parent.stdout.take().unwrap());
    let mut line = String::new();
    lines.read_line(&mut line).unwrap();
    let mut ready = String::new();
    lines.read_line(&mut ready).unwrap();
    assert_eq!(ready.trim(), "ready");
    let worker: i32 = line.trim().parse().unwrap();
    let alive = |pid: i32| {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .map(|stat| !stat.contains(") Z "))
            .unwrap_or(false)
    };
    assert!(alive(worker), "the worker runs while its parent lives");
    // SIGKILL models a harness dying with no chance to close the worker's
    // stdin.
    // SAFETY: the test owns the parent it spawned; constant signal, own pid.
    unsafe {
        libc::kill(parent.id() as libc::pid_t, libc::SIGKILL);
    }
    parent.wait().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    while alive(worker) {
        assert!(
            std::time::Instant::now() < deadline,
            "worker {worker} outlived its killed parent"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn interpreters_naming(marker: &str) -> Vec<u32> {
    std::fs::read_dir("/proc")
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let pid = entry.file_name().to_string_lossy().parse::<u32>().ok()?;
            let cmdline = std::fs::read(entry.path().join("cmdline")).ok()?;
            let stat = std::fs::read_to_string(entry.path().join("stat")).ok()?;
            (String::from_utf8_lossy(&cmdline).contains(marker) && !stat.contains(") Z "))
                .then_some(pid)
        })
        .collect()
}

/// An idle interpreter whose checkout has been removed leaves on its own
/// (fixtures and finished containers take their directories with them), so
/// nothing lingers to be counted as a stray of the harness.
#[test]
fn a_worker_leaves_once_its_checkout_is_gone() {
    let dir = checkout("vanish");
    let marker = dir.path().to_string_lossy().into_owned();
    call(dir.path(), "m", STUB, "echo", json!([])).unwrap();
    assert_eq!(interpreters_naming(&marker).len(), 1);
    drop(dir);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !interpreters_naming(&marker).is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "interpreter outlived its checkout"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
