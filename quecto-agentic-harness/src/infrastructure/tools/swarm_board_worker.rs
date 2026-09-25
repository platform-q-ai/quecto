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
//! stdin closes or its checkout directory disappears. Workers are kept in a
//! small process-wide LRU; eviction closes stdin and reaps the child. No
//! signal is ever sent by the harness. The packaged modules never write to
//! stdout or stderr themselves (stderr is read only after an exit), and a
//! call is never retried: an interpreter that vanished mid-call may already
//! have applied a mutating board method.
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
        // resident spawner below. It also leaves the harness's process group
        // (setsid), as bash children and members do, so a group-wide canary
        // never sees it. While idle it watches its own checkout every 100 ms
        // and leaves when that is gone; one request is outstanding at a time,
        // so selecting on the descriptor before each buffered readline is exact.
        let source = format!(
            "{PDEATHSIG}{bootstrap}\nimport os, select\n_checkout={checkout}\nwhile True:\n _ready,_,_=select.select([sys.stdin],[],[],0.1)\n if not _ready:\n  if not os.path.isdir(_checkout): break\n  continue\n _line=sys.stdin.readline()\n if not _line: break\n try:\n  _req=json.loads(_line); print(json.dumps({{'ok':getattr(swarm.board,_req[0])(*_req[1])}}))\n except Exception as e:\n  print(json.dumps({{'error':str(e)}}))\n sys.stdout.flush()\n",
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
        let mut bytes = Vec::new();
        if let Some(mut err) = self.child.stderr.take() {
            let _ = err.read_to_end(&mut bytes);
        }
        let _ = self.child.wait();
        String::from_utf8_lossy(&bytes).into_owned()
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

/// Prelude the interpreter runs first. `ctypes` may be absent on a minimal
/// python; then the worker degrades to exiting on stdin EOF / checkout loss.
/// The parent is re-checked after arming, as `parent_death_signal.rs` does:
/// a harness that died during interpreter start-up would otherwise never be
/// signalled.
const PDEATHSIG: &str = "import os\n_ppid=os.getppid()\ntry:\n os.setsid()\nexcept Exception:\n pass\ntry:\n import ctypes; ctypes.CDLL(None).prctl(1, ctypes.c_ulong(9))\n if os.getppid()!=_ppid: raise SystemExit(0)\nexcept SystemExit:\n raise\nexcept Exception:\n pass\n";

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

/// (checkout, database, member): a checkout whose board moved (the host
/// reads by layout, #2145) gets its own interpreter, never the old one's.
type Key = (PathBuf, PathBuf, String);
type Slot = Arc<Mutex<Option<Worker>>>;

/// The interpreters one process keeps, keyed by (checkout, database,
/// member).
#[derive(Default)]
pub(super) struct Registry {
    inner: Mutex<RegistryInner>,
}

#[derive(Default)]
struct RegistryInner {
    workers: HashMap<Key, (Slot, u64)>,
    tick: u64,
}

/// The process-wide registry every production caller shares.
fn global() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(Registry::default)
}

impl Registry {
    /// The slot for `(checkout, database, member)`, evicting the least recently used
    /// worker when full. Slots are created empty; the caller spawns into
    /// one under the slot's own lock, so the registry lock is never held
    /// across a spawn.
    fn slot(&self, checkout: &Path, database: &Path, member: &str) -> Slot {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.tick += 1;
        let tick = inner.tick;
        let key = (
            checkout.to_path_buf(),
            database.to_path_buf(),
            member.to_string(),
        );
        if let Some((slot, used)) = inner.workers.get_mut(&key) {
            *used = tick;
            return slot.clone();
        }
        if inner.workers.len() >= MAX_WORKERS {
            let oldest = inner
                .workers
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                let evicted = inner.workers.remove(&oldest);
                drop(inner);
                drop(evicted);
                return self.slot(checkout, database, member);
            }
        }
        let created: Slot = Arc::new(Mutex::new(None));
        inner.workers.insert(key, (created.clone(), tick));
        created
    }

    /// One board call against this registry's interpreter for
    /// `(checkout, member)`, started from `bootstrap` when absent or already
    /// exited. An interpreter that goes away mid-call is an error, never a
    /// retry (see the module doc); its stderr is the diagnostic.
    pub(super) fn call(
        &self,
        checkout: &Path,
        database: &Path,
        member: &str,
        bootstrap: &str,
        method: &str,
        args: Value,
    ) -> Result<Value, DomainError> {
        let request = serde_json::to_string(&serde_json::json!([method, args]))
            .map_err(|e| DomainError::Tool(format!("swarm coordination request: {e}")))?;
        let slot = self.slot(checkout, database, member);
        let mut guard = slot.lock().unwrap_or_else(|p| p.into_inner());
        // An interpreter that already left (its checkout vanished) is reaped
        // and replaced before the request is written, so the write never
        // targets a dead peer.
        if guard
            .as_mut()
            .is_some_and(|w| matches!(w.child.try_wait(), Ok(Some(_)) | Err(_)))
        {
            *guard = None;
        }
        let worker = match guard.as_mut() {
            Some(worker) => worker,
            None => guard.insert(Worker::spawn(checkout, bootstrap)?),
        };
        match worker.exchange(&request) {
            Some(line) => decode(&line),
            None => {
                let stderr = worker.stderr_after_exit();
                *guard = None;
                Err(DomainError::Tool(format!(
                    "swarm coordination failed: {stderr}"
                )))
            }
        }
    }

    #[cfg(test)]
    pub(super) fn resident(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .workers
            .len()
    }
}

/// One board call through the process-wide registry.
pub(super) fn call(
    checkout: &Path,
    database: &Path,
    member: &str,
    bootstrap: &str,
    method: &str,
    args: Value,
) -> Result<Value, DomainError> {
    global().call(checkout, database, member, bootstrap, method, args)
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
