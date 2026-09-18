//! A container script's stderr, kept for its failure report (#2024 S4b).
//!
//! Every script failure used to reach the model as `script-managed create
//! failed with status 1`: the runner sent the script's stderr to
//! `/dev/null`, so `die "image … is not present"` was lost and eight
//! create-then-rollback cycles in one afternoon left no clue. Each script
//! run now keeps the last [`STDERR_TAIL_CAPACITY`] bytes of its stderr —
//! bounded, so a script that floods cannot grow the report — sanitised of
//! terminal escapes and control characters, and appends them to the
//! failure text after the exit-status prefix every existing assertion
//! still matches.
use std::collections::VecDeque;
use std::io::Read;
use std::process::ExitStatus;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::infrastructure::processes::child_stderr_tail::{STDERR_TAIL_CAPACITY, StderrTail};

/// How long a report waits for the stderr pipe to reach EOF after the
/// script exited; a grandchild holding the pipe cannot stall it beyond.
const STDERR_REPORT_GRACE: Duration = Duration::from_millis(500);

/// A script's exit status, its whole stdout (the JSON contract) and the
/// sanitised tail of its stderr.
#[derive(Debug)]
pub struct ScriptOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr_tail: String,
}

impl ScriptOutput {
    /// `script-managed <operation> failed with status <status>[: <tail>]`:
    /// the historical prefix, then the script's last words when it had any.
    pub fn failure_message(&self, operation: &str) -> String {
        failure_message(operation, &self.status, &self.stderr_tail)
    }
}

pub fn failure_message(operation: &str, status: &ExitStatus, stderr_tail: &str) -> String {
    let mut message = format!("script-managed {operation} failed with status {status}");
    if !stderr_tail.is_empty() {
        message.push_str(": ");
        message.push_str(stderr_tail);
    }
    message
}

/// Whether a script's stdout is a result to read (the create/exec/inspect
/// JSON contract) or nothing (kill, cleanup): reading it to EOF waits for
/// every holder of the pipe, which only a contract-bearing stdout earns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptStdout {
    Result,
    Discard,
}

/// Run `cmd` to completion on the async runtime, keeping stdout whole
/// (when it is a result) and only the bounded tail of stderr. The report
/// waits at most [`STDERR_REPORT_GRACE`] after the exit for the stderr
/// pipe's EOF, so a grandchild holding it cannot stall the caller.
pub async fn run_capturing_stderr_tail(
    mut cmd: tokio::process::Command,
    stdout: ScriptStdout,
) -> std::io::Result<ScriptOutput> {
    cmd.stdout(match stdout {
        ScriptStdout::Result => std::process::Stdio::piped(),
        ScriptStdout::Discard => std::process::Stdio::null(),
    });
    cmd.stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn()?;
    let tail = child
        .stderr
        .take()
        .map(|stderr| StderrTail::pump(&tokio::runtime::Handle::current(), stderr));
    let output = child.wait_with_output().await?;
    let stderr_tail = match tail {
        Some(tail) => {
            tail.wait_eof(STDERR_REPORT_GRACE).await;
            sanitise(&tail.snapshot())
        }
        None => String::new(),
    };
    Ok(ScriptOutput {
        status: output.status,
        stdout: output.stdout,
        stderr_tail,
    })
}

/// Run `cmd` to completion on the calling (blocking) thread with a hard
/// wall-clock bound (`Duration::MAX` for none), keeping stdout whole
/// (when it is a result) and only the bounded tail of stderr. On timeout
/// the script is killed and the error names the bound and carries the
/// tail. After the exit the report waits at most [`STDERR_REPORT_GRACE`]
/// for each pipe's EOF: a grandchild holding a pipe open costs the grace,
/// never the caller's liveness (its pump thread ends when the pipe does).
pub fn run_sync_capturing_stderr_tail(
    mut cmd: std::process::Command,
    stdout: ScriptStdout,
    timeout: Duration,
) -> Result<ScriptOutput, String> {
    cmd.stdout(match stdout {
        ScriptStdout::Result => std::process::Stdio::piped(),
        ScriptStdout::Discard => std::process::Stdio::null(),
    });
    cmd.stderr(std::process::Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to invoke script: {e}"))?;
    // Both pipes are drained on their own threads so a script that fills
    // one while the poll loop waits on the other can never deadlock; each
    // reports through a channel so the wait after exit is bounded. A pump
    // whose pipe a grandchild keeps open outlives this call (blocked in
    // `read` until that holder exits): one thread per such script, never
    // the caller's liveness — a signal to the holder is not this runner's
    // to send (#1940).
    let stdout_pump = child.stdout.take().map(|mut pipe| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut stdout = Vec::new();
            let _ = pipe.read_to_end(&mut stdout);
            let _ = tx.send(stdout);
        });
        rx
    });
    let ring: Arc<Mutex<VecDeque<u8>>> =
        Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL_CAPACITY)));
    let stderr_pump = child.stderr.take().map(|pipe| {
        let (tx, rx) = std::sync::mpsc::channel();
        let sink = Arc::clone(&ring);
        std::thread::spawn(move || {
            drain_tail(pipe, &sink);
            let _ = tx.send(());
        });
        rx
    });
    let snapshot = |ring: &Mutex<VecDeque<u8>>| {
        let ring = ring.lock().unwrap_or_else(|e| e.into_inner());
        sanitise(&String::from_utf8_lossy(&Vec::from(ring.clone())))
    };
    let deadline = std::time::Instant::now().checked_add(timeout);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) => {
                let _ = child.kill();
                let _ = child.wait();
                if let Some(eof) = &stderr_pump {
                    let _ = eof.recv_timeout(STDERR_REPORT_GRACE);
                }
                let tail = snapshot(&ring);
                let mut message = format!(
                    "script timed out after {}s and was killed",
                    timeout.as_secs()
                );
                if !tail.is_empty() {
                    message.push_str(": ");
                    message.push_str(&tail);
                }
                return Err(message);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => return Err(format!("failed to reap script: {e}")),
        }
    };
    let stdout = stdout_pump
        .and_then(|rx| rx.recv_timeout(STDERR_REPORT_GRACE).ok())
        .unwrap_or_default();
    if let Some(eof) = &stderr_pump {
        let _ = eof.recv_timeout(STDERR_REPORT_GRACE);
    }
    let stderr_tail = snapshot(&ring);
    Ok(ScriptOutput {
        status,
        stdout,
        stderr_tail,
    })
}

/// Read `pipe` to EOF into `sink`, keeping only the last
/// [`STDERR_TAIL_CAPACITY`] bytes.
fn drain_tail(mut pipe: impl Read, sink: &Mutex<VecDeque<u8>>) {
    let mut buf = [0u8; 1024];
    loop {
        match pipe.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut ring = sink.lock().unwrap_or_else(|e| e.into_inner());
                for &byte in &buf[..n] {
                    if ring.len() == STDERR_TAIL_CAPACITY {
                        ring.pop_front();
                    }
                    ring.push_back(byte);
                }
                debug_assert!(ring.len() <= STDERR_TAIL_CAPACITY);
            }
        }
    }
}

/// The tail as a report may carry it: ANSI escape sequences removed,
/// `\r\n` folded to `\n`, every other control character dropped, tabs
/// widened to a space, surrounding whitespace trimmed. A script's colours
/// and cursor moves are for a terminal, not a tool result.
pub fn sanitise(tail: &str) -> String {
    let mut out = String::with_capacity(tail.len());
    let mut chars = tail.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => skip_escape_sequence(&mut chars),
            '\n' => out.push('\n'),
            '\t' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.trim().to_string()
}

/// Skip the body of an escape sequence: a CSI (`ESC [ … final`) whole, an
/// OSC (`ESC ] … BEL|ST`) up to its terminator or the end of the line (an
/// unterminated title must not swallow the rest of the report), and any
/// other sequence's intermediates (`0x20..=0x2f`) plus one final byte
/// (`ESC ( B`, `ESC = `).
fn skip_escape_sequence(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    match chars.next() {
        Some('[') => {
            // Parameter and intermediate bytes, then one final byte; a
            // newline (a sequence cut short) ends it without being eaten.
            while chars
                .next_if(|c| ('\u{20}'..='\u{3f}').contains(c))
                .is_some()
            {}
            chars.next_if(|c| ('\u{40}'..='\u{7e}').contains(c));
        }
        Some(']') => {
            while let Some(&c) = chars.peek() {
                if c == '\n' {
                    break;
                }
                chars.next();
                if c == '\u{7}' {
                    break;
                }
                if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                    chars.next();
                    break;
                }
            }
        }
        Some(c) if ('\u{20}'..='\u{2f}').contains(&c) => {
            while chars
                .next_if(|c| ('\u{20}'..='\u{2f}').contains(c))
                .is_some()
            {}
            chars.next();
        }
        _ => {}
    }
}

#[cfg(test)]
#[path = "script_stderr_tests.rs"]
mod tests;
