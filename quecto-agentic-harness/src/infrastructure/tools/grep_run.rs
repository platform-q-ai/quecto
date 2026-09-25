//! Running rg for the grep tool (#2136): stdout up to a cap and stderr
//! read at the same time, so rg never blocks on a full pipe; rg is killed at
//! the cap and after a timeout. rg starts no processes of its own: the tool
//! passes neither `--pre` nor `--search-zip`, and `--no-config` keeps a
//! user's rg config from adding them. Draining is bounded all the same: a
//! pipe still open once rg has exited (or stdout is done) gets a short
//! grace and is then abandoned.

use crate::domain::error::DomainError;

use super::MAX_OUTPUT_BYTES;

/// What one rg run printed: stdout up to the run's [`ReadLimit`] (`cut`
/// says why when more was left unread), the start of stderr, and the exit
/// code (`None` when the process was killed).
#[derive(Debug)]
pub(super) struct RgRun {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: Option<i32>,
    pub cut: Option<Cut>,
    /// stdout was still held open after rg exited, and abandoned.
    pub held_open: bool,
}

/// Why rg's output was not read to its end; rg is killed at each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Cut {
    /// The byte cap was reached.
    Bytes,
    /// The match cap (the number held) was reached: more matches followed.
    Matches(usize),
    /// rg did not finish within the timeout; what it printed stands.
    Timeout,
}

/// How much of rg's output is read: up to `bytes`, and, when `matches` is
/// set, no more than that many match records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ReadLimit {
    pub bytes: usize,
    pub matches: Option<usize>,
}

/// How much rg output a plain search reads (JSON is larger than the plain
/// text shown).
pub(super) const RG_STDOUT_CAP: usize = MAX_OUTPUT_BYTES * 4;
/// A ranked search's byte backstop (#2142): it reads up to as many matches
/// as it may judge, which for typical lines is well under this; very long
/// lines, context or many files are what reach it.
pub(super) const RANKED_STDOUT_CAP: usize = 16 * 1024 * 1024;
/// How each match record in rg's `--json` output starts (serde writes the
/// tag first).
const MATCH_RECORD: &[u8] = br#"{"type":"match""#;

/// How much of rg's stderr is kept (the rest is read and discarded).
const RG_STDERR_KEEP: usize = 4096;
/// How long a pipe may stay open once the other side is done (rg exited,
/// or stdout finished): then it is abandoned.
const PIPE_GRACE: std::time::Duration = std::time::Duration::from_secs(2);
/// How long one rg run may take before it is killed.
pub(super) const RG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Run rg, reading stdout (up to `limit`) and stderr at the same time, so
/// rg can never block on a full stderr pipe. At the limit rg is killed;
/// past `timeout` it is killed too, and what it printed stands (`Cut::Timeout`)
/// or, with nothing printed, the search is reported as too slow.
pub(super) async fn run_rg(
    mut cmd: tokio::process::Command,
    timeout: std::time::Duration,
    limit: ReadLimit,
) -> Result<RgRun, DomainError> {
    assert!(limit.bytes > 0, "rg's output is read up to a positive cap");
    assert!(
        limit.matches.is_none_or(|matches| matches > 0),
        "a match cap reads at least one match"
    );
    cmd.kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            DomainError::Tool(
                "rg not found on PATH — install ripgrep: https://github.com/BurntSushi/ripgrep#installation".to_string()
            )
        } else {
            DomainError::Tool(format!("grep failed to spawn rg: {}", e))
        }
    })?;
    let stdout = child.stdout.take();
    // stderr drains in its own task, so neither pipe can block rg; it is
    // aborted when this call ends, however it ends (the guard's drop).
    let stderr_task = tokio::spawn(read_head(child.stderr.take(), RG_STDERR_KEEP));
    let _abort_stderr = AbortOnDrop(stderr_task.abort_handle());
    let mut out = Vec::with_capacity(limit.bytes.min(64 * 1024));
    let run = async {
        let (cut, exited) = {
            let read = read_limited(stdout, limit, &mut out);
            tokio::pin!(read);
            tokio::select! {
                cut = &mut read => (cut, None),
                // rg exited while stdout is still open: whatever holds it
                // (not rg, which starts nothing) gets a grace, no more.
                status = child.wait() => {
                    match tokio::time::timeout(PIPE_GRACE, &mut read).await {
                        Ok(cut) => (cut, Some((status, false))),
                        Err(_) => (None, Some((status, true))),
                    }
                }
            }
        };
        if cut.is_some() {
            // Nothing more will be read: end rg.
            let _ = child.start_kill();
        }
        let stderr = match tokio::time::timeout(PIPE_GRACE, stderr_task).await {
            Ok(Ok(kept)) => kept,
            Ok(Err(_)) | Err(_) => Vec::new(),
        };
        let (status, held_open) = match exited {
            Some((status, held_open)) => (status, held_open),
            None => (child.wait().await, false),
        };
        (cut, held_open, stderr, status)
    };
    let outcome = tokio::time::timeout(timeout, run).await;
    match outcome {
        Ok((cut, held_open, stderr, status)) => Ok(RgRun {
            stdout: out,
            stderr,
            exit_code: status.ok().and_then(|s| s.code()),
            cut,
            held_open,
        }),
        Err(_) => {
            // Signal only: an rg stuck in uninterruptible I/O (a hung mount)
            // must not hold the tool; tokio reaps the dropped Child.
            let _ = child.start_kill();
            match out.is_empty() {
                // What was printed before the timeout stands, marked cut.
                false => Ok(RgRun {
                    stdout: out,
                    stderr: Vec::new(),
                    exit_code: None,
                    cut: Some(Cut::Timeout),
                    held_open: false,
                }),
                true => Err(DomainError::Tool(format!(
                    "rg did not finish within {}: narrow the search with path, glob or type",
                    human_duration(timeout)
                ))),
            }
        }
    }
}

/// Read `pipe` to EOF or to `limit`; the cut when more was left unread.
/// At a match cap `bytes` ends just before the first match past it.
async fn read_limited(
    pipe: Option<tokio::process::ChildStdout>,
    limit: ReadLimit,
    bytes: &mut Vec<u8>,
) -> Option<Cut> {
    use tokio::io::AsyncReadExt;
    let mut pipe = pipe?;
    let mut lines = MatchLines::default();
    let mut buf = vec![0u8; 8192];
    loop {
        let n = pipe.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            return None;
        }
        let take = n.min(limit.bytes.saturating_sub(bytes.len()));
        bytes.extend_from_slice(&buf[..take]);
        if let Some(max) = limit.matches
            && let Some(past) = lines.find_past(bytes, max)
        {
            bytes.truncate(past);
            return Some(Cut::Matches(max));
        }
        if take < n {
            return Some(Cut::Bytes);
        }
        if bytes.len() >= limit.bytes {
            let more = pipe.read(&mut buf).await.is_ok_and(|more| more > 0);
            return more.then_some(Cut::Bytes);
        }
    }
}

/// Counts the match records among the whole lines read so far.
#[derive(Debug, Default)]
struct MatchLines {
    /// Where the first line not yet counted starts.
    next: usize,
    matches: usize,
}

impl MatchLines {
    /// Count the whole lines added since the last call; the start of the
    /// first match record past `max`, if one has been read.
    fn find_past(&mut self, bytes: &[u8], max: usize) -> Option<usize> {
        while let Some(end) = bytes[self.next..].iter().position(|b| *b == b'\n') {
            let start = self.next;
            self.next = start + end + 1;
            if bytes[start..].starts_with(MATCH_RECORD) {
                self.matches += 1;
                if self.matches > max {
                    return Some(start);
                }
            }
        }
        None
    }
}

/// Aborts a task when dropped, so it never outlives the call that owns it.
struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Read `pipe` to EOF, keeping the first `keep` bytes.
async fn read_head(pipe: Option<tokio::process::ChildStderr>, keep: usize) -> Vec<u8> {
    use tokio::io::AsyncReadExt;
    let mut kept = Vec::new();
    let Some(mut pipe) = pipe else {
        return kept;
    };
    let mut buf = vec![0u8; 4096];
    loop {
        let n = pipe.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            return kept;
        }
        let take = n.min(keep.saturating_sub(kept.len()));
        kept.extend_from_slice(&buf[..take]);
    }
}

/// `60 s`, or `300 ms` below a second.
pub(super) fn human_duration(duration: std::time::Duration) -> String {
    if duration.as_secs() >= 1 {
        format!("{} s", duration.as_secs())
    } else {
        format!("{} ms", duration.as_millis())
    }
}
