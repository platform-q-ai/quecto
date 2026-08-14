use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use crate::domain::error::DomainError;
use crate::infrastructure::tools::python_lab::{JobState, RunSpec};

pub(crate) async fn run_child(
    spec: RunSpec,
    workspace: &Path,
    stdout_path: &Path,
    stderr_path: &Path,
    state: Option<Arc<Mutex<JobState>>>,
) -> Result<(String, Option<i32>), DomainError> {
    let mut cmd = Command::new("python3");
    cmd.current_dir(workspace).kill_on_drop(true);
    if spec.inherit_environment {
        cmd.env("PYTHONNOUSERSITE", "1");
    } else {
        cmd.env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("PYTHONNOUSERSITE", "1");
    }
    apply_child_limits(&mut cmd, &spec);
    if let Some(src) = spec.code {
        cmd.arg("-c").arg(src);
    } else {
        cmd.arg("--").arg(spec.script.unwrap());
    }
    for a in spec.args {
        cmd.arg(a);
    }
    if spec.stdin.is_some() {
        cmd.stdin(std::process::Stdio::piped());
    }
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| DomainError::Other(format!("failed to start python3: {e}")))?;
    let stdout_task = child
        .stdout
        .take()
        .map(|pipe| tokio::spawn(copy_output(pipe, stdout_path.to_path_buf(), spec.max_out)));
    let stderr_task = child
        .stderr
        .take()
        .map(|pipe| tokio::spawn(copy_output(pipe, stderr_path.to_path_buf(), spec.max_out)));
    if let Some(st) = &state {
        if let Ok(mut s) = st.lock() {
            s.pid = child.id();
            if s.cancel_requested {
                if let Some(pid) = s.pid {
                    kill_pid(pid);
                }
            }
        }
    }
    if let Some(input) = spec.stdin {
        if let Some(mut pipe) = child.stdin.take() {
            tokio::spawn(async move {
                let _ = pipe.write_all(input.as_bytes()).await;
            });
        }
    }
    let wait = child.wait();
    let outcome = match tokio::time::timeout(Duration::from_secs(spec.timeout_secs), wait).await {
        Ok(Ok(st)) => Ok(("completed".into(), st.code())),
        Ok(Err(e)) => Err(DomainError::Other(format!("python3 execution failed: {e}"))),
        Err(_) => {
            if let Some(pid) = child.id() {
                kill_pid(pid);
                kill_pid_tree_best_effort(pid);
            } else {
                let _ = child.kill().await;
            }
            let _ = child.wait().await;
            Ok(("timed_out".into(), None))
        }
    };
    let drain_timeout = if matches!(outcome, Ok((ref st, _)) if st == "timed_out")
        || state
            .as_ref()
            .and_then(|st| st.lock().ok().map(|s| s.cancel_requested))
            .unwrap_or(false)
    {
        Duration::from_millis(500)
    } else {
        Duration::from_secs(5)
    };
    if let Some(task) = stdout_task {
        let _ = tokio::time::timeout(drain_timeout, task).await;
    }
    if let Some(task) = stderr_task {
        let _ = tokio::time::timeout(drain_timeout, task).await;
    }
    outcome
}

async fn copy_output<R>(mut reader: R, path: PathBuf, max_bytes: usize)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let Ok(mut out) = tokio::fs::File::create(path).await else {
        return;
    };
    let mut remaining = max_bytes;
    let mut buf = [0_u8; 8192];
    while remaining > 0 {
        let cap = buf.len().min(remaining);
        let n = match reader.read(&mut buf[..cap]).await {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        if out.write_all(&buf[..n]).await.is_err() {
            return;
        }
        remaining -= n;
    }
}

#[cfg(unix)]
pub(crate) fn kill_pid(pid: u32) {
    // SAFETY: libc::kill is called with an OS pid obtained from tokio::process::Child::id.
    unsafe {
        let target = -(pid as i32);
        if libc::kill(target, libc::SIGKILL) != 0 {
            libc::kill(pid as i32, libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
pub(crate) fn kill_pid(_pid: u32) {}

#[cfg(unix)]
fn apply_child_limits(cmd: &mut Command, spec: &RunSpec) {
    let memory = spec.max_memory_bytes;
    let cpu = spec.max_cpu_seconds;
    let procs = spec.max_processes;
    // pre_exec only performs async-signal-safe libc calls to set process group
    // and resource limits before exec; captured values are plain integers/options.
    // SAFETY: see above.
    unsafe {
        cmd.pre_exec(move || {
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if let Some(bytes) = memory {
                let lim = libc::rlimit {
                    rlim_cur: bytes as libc::rlim_t,
                    rlim_max: bytes as libc::rlim_t,
                };
                if libc::setrlimit(libc::RLIMIT_AS, &lim) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            if let Some(seconds) = cpu {
                let lim = libc::rlimit {
                    rlim_cur: seconds as libc::rlim_t,
                    rlim_max: seconds as libc::rlim_t,
                };
                if libc::setrlimit(libc::RLIMIT_CPU, &lim) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            if let Some(processes) = procs {
                let lim = libc::rlimit {
                    rlim_cur: processes as libc::rlim_t,
                    rlim_max: processes as libc::rlim_t,
                };
                if libc::setrlimit(libc::RLIMIT_NPROC, &lim) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn apply_child_limits(_cmd: &mut Command, _spec: &RunSpec) {}

#[cfg(unix)]
pub(crate) fn kill_pid_tree_best_effort(pid: u32) {
    let Ok(out) = std::process::Command::new("pgrep")
        .arg("-P")
        .arg(pid.to_string())
        .output()
    else {
        return;
    };
    for child in String::from_utf8_lossy(&out.stdout).lines() {
        if let Ok(child_pid) = child.trim().parse::<u32>() {
            kill_pid_tree_best_effort(child_pid);
            kill_pid(child_pid);
        }
    }
}

#[cfg(not(unix))]
pub(crate) fn kill_pid_tree_best_effort(_pid: u32) {}
