//! Persistent coordination-board interpreters. Every board call used to start
//! a fresh `python3 -I`, compile the packaged swarm modules and open the store
//! (~100 ms of CPU per call; ~0.4 s under a loaded test run). The board is
//! stateless between calls — `Store` opens one SQLite connection per
//! transaction and `Workbench` keeps only its path, checkout and member — so
//! one interpreter per (checkout, member) serving calls over stdin/stdout is
//! observably the same board with the start-up paid once.
//!
//! Protocol: one JSON line `[method, args]` in, one JSON line
//! `{"ok": value}` or `{"error": text}` out; the interpreter exits when its
//! stdin closes. Workers are kept in a small process-wide LRU; eviction closes
//! stdin and reaps the child. No signal is ever sent.
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock, mpsc};

use serde_json::Value;

use crate::domain::error::DomainError;

/// Idle interpreters kept alive per process: a member talks to one board;
/// the supervising session and the test suites touch a handful.
const MAX_WORKERS: usize = 8;

/// An interpreter idle this long exits on its own; a later call restarts one
/// (paying the start-up once, as every call used to).
const IDLE_EXIT_SECS: u32 = 10;

struct Worker {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Worker {
    fn spawn(checkout: &Path, bootstrap: &str) -> Result<Self, DomainError> {
        // The interpreter asks the kernel to end it with its parent
        // (PR_SET_PDEATHSIG): the harness never signals it, and a harness that
        // dies mid-idle leaves no interpreter behind for the exit canary to
        // name. The signal is bound to the spawning *thread*, hence the
        // resident spawner below. While idle it watches its own checkout and
        // leaves when that is gone or after IDLE_EXIT_SECS; the next call
        // simply starts a fresh one. One request is outstanding at a time, so
        // selecting on the descriptor before each buffered readline is exact.
        let source = format!(
            "{PDEATHSIG}{bootstrap}\nimport os, select, time\n_checkout={checkout}\n_idle=time.monotonic()\nwhile True:\n _ready,_,_=select.select([sys.stdin],[],[],0.1)\n if not _ready:\n  if not os.path.isdir(_checkout) or time.monotonic()-_idle>{IDLE_EXIT_SECS}: break\n  continue\n _line=sys.stdin.readline()\n if not _line: break\n _idle=time.monotonic()\n try:\n  _req=json.loads(_line); print(json.dumps({{'ok':getattr(swarm.board,_req[0])(*_req[1])}}))\n except Exception as e:\n  print(json.dumps({{'error':str(e)}}))\n sys.stdout.flush()\n",
            checkout = serde_json::json!(checkout.to_string_lossy()),
        );
        let mut child = spawn_on_resident_thread(source)
            .map_err(|e| DomainError::Tool(format!("swarm coordination interpreter: {e}")))?;
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
        Ok(Self {
            child,
            stdin,
            stdout,
        })
    }

    /// One round trip; `None` when the interpreter has gone away (EOF or a
    /// closed pipe), in which case the caller decides whether to retry.
    fn exchange(&mut self, request: &str) -> Option<String> {
        let stdin = self.stdin.as_mut()?;
        stdin.write_all(request.as_bytes()).ok()?;
        stdin.write_all(b"\n").ok()?;
        stdin.flush().ok()?;
        let mut line = String::new();
        match self.stdout.read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line),
        }
    }

    /// Whatever the interpreter wrote to stderr before it exited.
    fn stderr_after_exit(&mut self) -> String {
        self.stdin.take();
        let mut text = String::new();
        if let Some(mut err) = self.child.stderr.take() {
            let _ = err.read_to_string(&mut text);
        }
        let _ = self.child.wait();
        text
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Closing stdin ends the interpreter's read loop; reap it so no zombie
        // outlives the registry entry.
        self.stdin.take();
        let _ = self.child.wait();
    }
}

const PDEATHSIG: &str =
    "try:\n import ctypes; ctypes.CDLL(None).prctl(1, 9)\nexcept Exception:\n pass\n";

/// Spawns interpreters from one thread that lives as long as the process, so
/// their parent-death signal fires at process exit and not when a pooled
/// worker thread retires.
fn spawn_on_resident_thread(source: String) -> std::io::Result<Child> {
    type Request = (String, mpsc::Sender<std::io::Result<Child>>);
    static SPAWNER: OnceLock<Mutex<mpsc::Sender<Request>>> = OnceLock::new();
    let spawner = SPAWNER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Request>();
        std::thread::Builder::new()
            .name("swarm-board-spawner".into())
            .spawn(move || {
                for (source, reply) in rx {
                    let _ = reply.send(
                        Command::new("python3")
                            .args(["-I", "-c", &source])
                            .env_clear()
                            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
                            .stdin(Stdio::piped())
                            .stdout(Stdio::piped())
                            .stderr(Stdio::piped())
                            .spawn(),
                    );
                }
            })
            .expect("spawn the swarm board spawner thread");
        Mutex::new(tx)
    });
    let (reply_tx, reply_rx) = mpsc::channel();
    spawner
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .send((source, reply_tx))
        .map_err(|_| std::io::Error::other("swarm board spawner thread is gone"))?;
    reply_rx
        .recv()
        .map_err(|_| std::io::Error::other("swarm board spawner thread is gone"))?
}

type Key = (PathBuf, String);
type Slot = Arc<Mutex<Option<Worker>>>;

#[derive(Default)]
struct Registry {
    workers: HashMap<Key, (Slot, u64)>,
    tick: u64,
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(Mutex::default)
}

/// The slot for `(checkout, member)`, evicting the least recently used
/// worker when the registry is full. The slot is created empty; the caller
/// spawns into it under the slot's own lock so the registry lock is never
/// held across a spawn.
fn slot(checkout: &Path, member: &str) -> Slot {
    let mut registry = registry().lock().unwrap_or_else(|p| p.into_inner());
    registry.tick += 1;
    let tick = registry.tick;
    let key = (checkout.to_path_buf(), member.to_string());
    if let Some((slot, used)) = registry.workers.get_mut(&key) {
        *used = tick;
        return slot.clone();
    }
    if registry.workers.len() >= MAX_WORKERS {
        if let Some(oldest) = registry
            .workers
            .iter()
            .min_by_key(|(_, (_, used))| *used)
            .map(|(key, _)| key.clone())
        {
            let evicted = registry.workers.remove(&oldest);
            drop(registry);
            drop(evicted);
            return slot(checkout, member);
        }
    }
    let created = Arc::new(Mutex::new(None));
    registry.workers.insert(key, (created.clone(), tick));
    created
}

/// One board call against the interpreter for `(checkout, member)`, starting
/// it from `bootstrap` when absent. An interpreter that has exited is started
/// again once; if that also fails the interpreter's stderr is the error.
pub(super) fn call(
    checkout: &Path,
    member: &str,
    bootstrap: &str,
    method: &str,
    args: Value,
) -> Result<Value, DomainError> {
    let request = serde_json::to_string(&serde_json::json!([method, args]))
        .map_err(|e| DomainError::Tool(format!("swarm coordination request: {e}")))?;
    let slot = slot(checkout, member);
    let mut guard = slot.lock().unwrap_or_else(|p| p.into_inner());
    for attempt in 0..2 {
        let worker = match guard.as_mut() {
            Some(worker) => worker,
            None => guard.insert(Worker::spawn(checkout, bootstrap)?),
        };
        if let Some(line) = worker.exchange(&request) {
            return decode(&line);
        }
        let stderr = worker.stderr_after_exit();
        *guard = None;
        if attempt == 1 || !stderr.is_empty() {
            return Err(DomainError::Tool(format!(
                "swarm coordination failed: {stderr}"
            )));
        }
    }
    unreachable!("two attempts return or fail")
}

#[cfg(test)]
pub(super) fn resident() -> usize {
    registry()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .workers
        .len()
}

fn decode(line: &str) -> Result<Value, DomainError> {
    let value: Value = serde_json::from_str(line)
        .map_err(|e| DomainError::Tool(format!("invalid swarm coordination response: {e}")))?;
    if let Some(error) = value.get("error") {
        return Err(DomainError::Tool(format!("swarm: {error}")));
    }
    Ok(value["ok"].clone())
}

#[cfg(test)]
#[path = "swarm_board_worker_tests.rs"]
mod tests;
