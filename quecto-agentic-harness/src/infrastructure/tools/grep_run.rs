//! Running rg for the grep tool (#2136): stdout up to a cap and stderr
//! read at the same time, so rg never blocks on a full pipe; rg is killed at
//! the cap and after a timeout. rg starts no processes of its own: the tool
//! passes neither `--pre` nor `--search-zip`, and `--no-config` keeps a
//! user's rg config from adding them. Draining is bounded all the same.

use crate::domain::error::DomainError;

use super::MAX_OUTPUT_BYTES;

/// What one rg run printed: stdout up to [`RG_STDOUT_CAP`] (`capped` when
/// more was cut off), the start of stderr, and the exit code (`None` when
/// the process was killed).
#[derive(Debug)]
pub(super) struct RgRun {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: Option<i32>,
    pub capped: bool,
}

/// How much rg output is read (JSON is larger than the plain text shown).
pub(super) const RG_STDOUT_CAP: usize = MAX_OUTPUT_BYTES * 4;

/// How much of rg's stderr is kept (the rest is read and discarded).
const RG_STDERR_KEEP: usize = 4096;
/// How long stderr may stay open once stdout is done.
const STDERR_GRACE: std::time::Duration = std::time::Duration::from_secs(2);
/// How long one rg run may take before it is killed.
pub(super) const RG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Run rg, reading stdout (up to the cap) and stderr at the same time, so
/// rg can never block on a full stderr pipe. At the cap rg is killed; past
/// `timeout` it is killed and the search reported as too slow.
pub(super) async fn run_rg(
    mut cmd: tokio::process::Command,
    timeout: std::time::Duration,
) -> Result<RgRun, DomainError> {
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
    // stderr drains in its own task, so neither pipe can block rg; once
    // stdout is done it gets a short grace and is then abandoned, so
    // nothing still holding the pipe can keep the results waiting.
    let stderr_task = tokio::spawn(read_head(child.stderr.take(), RG_STDERR_KEEP));
    let abort_stderr = stderr_task.abort_handle();
    let run = async {
        let (stdout, capped) = read_capped(stdout, RG_STDOUT_CAP).await;
        if capped {
            // Nothing more will be read: end rg.
            let _ = child.start_kill();
        }
        let stderr = match tokio::time::timeout(STDERR_GRACE, stderr_task).await {
            Ok(Ok(kept)) => kept,
            Ok(Err(_)) | Err(_) => Vec::new(),
        };
        let status = child.wait().await;
        ((stdout, capped), stderr, status)
    };
    let outcome = tokio::time::timeout(timeout, run).await;
    abort_stderr.abort();
    match outcome {
        Ok(((stdout, capped), stderr, status)) => Ok(RgRun {
            stdout,
            stderr,
            exit_code: status.ok().and_then(|s| s.code()),
            capped,
        }),
        Err(_) => {
            // Signal only: an rg stuck in uninterruptible I/O (a hung mount)
            // must not hold the tool; tokio reaps the dropped Child.
            let _ = child.start_kill();
            Err(DomainError::Tool(format!(
                "rg did not finish within {}: narrow the search with path, glob or type",
                human_duration(timeout)
            )))
        }
    }
}

/// Read `pipe` to EOF or `cap` bytes; `true` when more was left unread.
async fn read_capped(pipe: Option<tokio::process::ChildStdout>, cap: usize) -> (Vec<u8>, bool) {
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::with_capacity(cap.min(64 * 1024));
    let Some(mut pipe) = pipe else {
        return (bytes, false);
    };
    let mut buf = vec![0u8; 8192];
    loop {
        let n = pipe.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            return (bytes, false);
        }
        let take = n.min(cap.saturating_sub(bytes.len()));
        bytes.extend_from_slice(&buf[..take]);
        if take < n {
            return (bytes, true);
        }
        if bytes.len() >= cap {
            let more = pipe.read(&mut buf).await.is_ok_and(|more| more > 0);
            return (bytes, more);
        }
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
fn human_duration(duration: std::time::Duration) -> String {
    if duration.as_secs() >= 1 {
        format!("{} s", duration.as_secs())
    } else {
        format!("{} ms", duration.as_millis())
    }
}
