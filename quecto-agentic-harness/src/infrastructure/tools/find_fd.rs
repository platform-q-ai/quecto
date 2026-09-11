//! fd discovery effect. Process and pipe ownership stay outside the use case.
use crate::application::agent_turn::use_cases::find::{
    FindError, FindOutput, FindPaths, FindPathsRequest,
};
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::path_utils::resolve_to_cwd;
use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, Command},
    sync::oneshot,
};

const STDOUT_CAP: usize = crate::domain::constants::DEFAULT_OUTPUT_CAP_BYTES * 2;
const STDERR_CAP: usize = 4096;

pub struct FdFindPaths {
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
    binary: String,
}
impl FdFindPaths {
    pub fn new(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>) -> Self {
        Self {
            workspace,
            sandbox,
            binary: "fd".into(),
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_fd_binary(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>, binary: String) -> Self {
        Self {
            workspace,
            sandbox,
            binary,
        }
    }
}
impl FindPaths for FdFindPaths {
    fn find(
        &self,
        request: FindPathsRequest,
    ) -> Pin<Box<dyn Future<Output = Result<FindOutput, FindError>> + Send + '_>> {
        Box::pin(async move {
            let root = resolve_to_cwd(&request.path, &self.workspace);
            self.sandbox
                .validate_path(&root.to_string_lossy())
                .map_err(|e| FindError::Security(e.to_string()))?;
            let mut command = self.command(&request, &root);
            let (sender, receiver) = oneshot::channel();
            // The owner has its own thread and reactor: shutting down the caller's
            // runtime cannot abort termination/wait. Receiver closure cancels the
            // invocation; the owner runtime lives until the child has been reaped.
            // Both thread and reactor creation happen before any child is spawned.
            std::thread::Builder::new()
                .name("find-fd-owner".into())
                .spawn(move || {
                    match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(runtime) => {
                            runtime.block_on(own_process(&mut command, root, request.limit, sender))
                        }
                        Err(error) => {
                            let _ = sender.send(Err(FindError::Io(format!(
                                "find failed to initialize process owner: {error}"
                            ))));
                        }
                    }
                })
                .map_err(|error| {
                    FindError::Io(format!("find failed to start process owner: {error}"))
                })?;
            receiver
                .await
                .map_err(|e| FindError::Io(format!("find process owner failed: {e}")))?
        })
    }
}
impl FdFindPaths {
    fn command(&self, request: &FindPathsRequest, root: &Path) -> Command {
        let mut command = Command::new(&self.binary);
        command
            .current_dir(self.workspace.as_ref())
            .args([
                "--glob",
                "--color=never",
                "--hidden",
                "--no-require-git",
                "--max-results",
            ])
            .arg(request.limit.to_string());
        let pattern = if request.pattern.contains('/') {
            command.arg("--full-path");
            let normalized = request
                .pattern
                .strip_prefix("./")
                .or_else(|| request.pattern.strip_prefix('/'))
                .unwrap_or(&request.pattern);
            if normalized.starts_with("**") {
                normalized.to_owned()
            } else {
                format!("**/{normalized}")
            }
        } else {
            request.pattern.clone()
        };
        command
            .arg("--")
            .arg(pattern)
            .arg(root)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        command
    }
}

async fn own_process(
    command: &mut Command,
    root: PathBuf,
    limit: usize,
    mut sender: oneshot::Sender<Result<FindOutput, FindError>>,
) {
    match sender.is_closed() {
        false => {}
        true => return,
    }
    let result = match command.spawn() {
        Ok(mut child) => {
            let outcome = tokio::select! {
                biased;
                _ = sender.closed() => None,
                result = collect(&mut child, &root, limit) => Some(result),
            };
            match outcome {
                Some(result) => result,
                None => {
                    let _ = terminate(&mut child).await;
                    return;
                }
            }
        }
        Err(error) => Err(spawn_error(error)),
    };
    let _ = sender.send(result);
}

fn spawn_error(error: std::io::Error) -> FindError {
    let message = match error.kind() {
        std::io::ErrorKind::NotFound => {
            "fd not found on PATH — install fd-find: https://github.com/sharkdp/fd#installation"
                .into()
        }
        _ => format!("find failed to spawn fd: {error}"),
    };
    FindError::Spawn(message)
}

async fn terminate(child: &mut Child) -> Result<(), FindError> {
    // start_kill does not reap. Always wait, including a process that exited
    // between the stop decision and signal delivery.
    let kill = child.start_kill();
    let wait = child.wait().await;
    match wait {
        Ok(_) => Ok(()),
        Err(error) => Err(FindError::Io(format!(
            "find failed to reap fd: {error}; kill: {kill:?}"
        ))),
    }
}

struct Capture {
    bytes: Vec<u8>,
    capped: bool,
}
async fn drain(
    reader: &mut (impl AsyncRead + Unpin),
    cap: usize,
    bytes: &mut Vec<u8>,
) -> std::io::Result<bool> {
    let mut buffer = [0; 8192];
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            return Ok(false);
        }
        let retained = count.min(cap.saturating_sub(bytes.len()));
        bytes.extend_from_slice(&buffer[..retained]);
        assert!(bytes.len() <= cap);
        if bytes.len() == cap {
            return Ok(true);
        }
    }
}

async fn collect(child: &mut Child, root: &Path, limit: usize) -> Result<FindOutput, FindError> {
    let mut stdout = child.stdout.take().expect("piped stdout invariant");
    let mut stderr = child.stderr.take().expect("piped stderr invariant");
    let mut out = Capture {
        bytes: Vec::new(),
        capped: false,
    };
    let mut err = Capture {
        bytes: Vec::new(),
        capped: false,
    };
    // A cap is an intentional stop, not EOF. Returning either cap immediately
    // drops both read futures before termination; never wait on another pipe.
    let streams: std::io::Result<()> = async {
        let read_out = drain(&mut stdout, STDOUT_CAP, &mut out.bytes);
        let read_err = drain(&mut stderr, STDERR_CAP, &mut err.bytes);
        tokio::pin!(read_out, read_err);
        tokio::select! {
            result = &mut read_out => {
                out.capped = result?;
                if out.capped { Ok(()) } else { err.capped = read_err.await?; Ok(()) }
            }
            result = &mut read_err => {
                err.capped = result?;
                if err.capped { Ok(()) } else { out.capped = read_out.await?; Ok(()) }
            }
        }
    }
    .await;
    finish_read(child, streams).await?;
    if out.capped || err.capped {
        terminate(child).await?;
        return Ok(output(&out.bytes, &err.bytes, root, limit, true));
    }
    let waited = child.wait().await;
    let status = finish_wait(child, waited).await?;
    classify(status.code(), &out.bytes, &err.bytes, root, limit)
}

async fn finish_read(child: &mut Child, read: std::io::Result<()>) -> Result<(), FindError> {
    match read {
        Ok(()) => Ok(()),
        Err(error) => {
            let cleanup = terminate(child).await;
            Err(FindError::Io(format!(
                "find failed reading fd output: {error}; cleanup: {cleanup:?}"
            )))
        }
    }
}

async fn finish_wait(
    child: &mut Child,
    waited: std::io::Result<std::process::ExitStatus>,
) -> Result<std::process::ExitStatus, FindError> {
    match waited {
        Ok(status) => Ok(status),
        Err(error) => {
            let cleanup = terminate(child).await;
            Err(FindError::Io(format!(
                "find failed waiting for fd: {error}; cleanup: {cleanup:?}"
            )))
        }
    }
}

fn diagnostic(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    if text.trim().is_empty() {
        "fd exited unexpectedly".into()
    } else {
        format!("find error: {}", text.trim())
    }
}
fn classify(
    code: Option<i32>,
    stdout: &[u8],
    stderr: &[u8],
    root: &Path,
    limit: usize,
) -> Result<FindOutput, FindError> {
    match code {
        Some(0 | 1) => Ok(output(stdout, &[], root, limit, false)),
        Some(2) => Err(FindError::Search(diagnostic(stderr))),
        _ if stdout.is_empty() => Err(FindError::Search(diagnostic(stderr))),
        _ => {
            let mut result = output(stdout, stderr, root, limit, false);
            result.incomplete = true;
            result.diagnostic = Some(diagnostic(stderr));
            Ok(result)
        }
    }
}
fn output(stdout: &[u8], stderr: &[u8], root: &Path, limit: usize, stopped: bool) -> FindOutput {
    let entries = normalize_output(stdout, root, stopped);
    FindOutput {
        result_limit_reached: entries.len() >= limit,
        entries,
        incomplete: stopped,
        diagnostic: if stopped && stderr.is_empty() {
            None
        } else if stopped {
            Some(diagnostic(stderr))
        } else {
            None
        },
    }
}
fn normalize_output(bytes: &[u8], root: &Path, stopped: bool) -> Vec<String> {
    let complete = if stopped {
        bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(&bytes[..0], |last| &bytes[..=last])
    } else {
        bytes
    };
    let raw = String::from_utf8_lossy(complete);
    if raw.trim().is_empty() {
        return Vec::new();
    }
    let root = root.to_string_lossy();
    let root = root.trim_end_matches('/');
    let prefix = format!("{root}/");
    raw.lines()
        .filter_map(|line| match line {
            "" => None,
            entry => Some(entry.strip_prefix(&prefix).unwrap_or(entry).to_owned()),
        })
        .collect()
}

#[cfg(test)]
#[path = "find_fd_tests.rs"]
mod tests;
