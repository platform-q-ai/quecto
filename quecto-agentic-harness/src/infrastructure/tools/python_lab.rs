use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;

#[derive(Debug, Clone)]
pub struct PythonLabConfig {
    pub default_timeout_seconds: u64,
    pub max_foreground_seconds: u64,
    pub max_background_seconds: u64,
    pub default_max_output_bytes: usize,
    pub max_output_bytes: usize,
    pub max_memory_bytes: Option<u64>,
    pub max_cpu_seconds: Option<u64>,
    pub max_processes: Option<u32>,
    pub max_concurrent_jobs: usize,
    pub inherit_environment: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PythonLabToolConfig {
    #[serde(default = "default_python_lab_timeout_seconds")]
    pub default_timeout_seconds: u64,
    #[serde(default = "default_python_lab_max_foreground_seconds")]
    pub max_foreground_seconds: u64,
    #[serde(default = "default_python_lab_max_background_seconds")]
    pub max_background_seconds: u64,
    #[serde(default = "default_python_lab_output_bytes")]
    pub default_max_output_bytes: usize,
    #[serde(default = "default_python_lab_max_output_bytes")]
    pub max_output_bytes: usize,
    #[serde(default)]
    pub max_memory_bytes: Option<u64>,
    #[serde(default)]
    pub max_cpu_seconds: Option<u64>,
    #[serde(default = "default_python_lab_max_processes")]
    pub max_processes: Option<u32>,
    #[serde(default = "default_python_lab_concurrent_jobs")]
    pub max_concurrent_jobs: usize,
    #[serde(default)]
    pub inherit_environment: bool,
}

impl Default for PythonLabToolConfig {
    fn default() -> Self {
        Self {
            default_timeout_seconds: default_python_lab_timeout_seconds(),
            max_foreground_seconds: default_python_lab_max_foreground_seconds(),
            max_background_seconds: default_python_lab_max_background_seconds(),
            default_max_output_bytes: default_python_lab_output_bytes(),
            max_output_bytes: default_python_lab_max_output_bytes(),
            max_memory_bytes: None,
            max_cpu_seconds: None,
            max_processes: default_python_lab_max_processes(),
            max_concurrent_jobs: default_python_lab_concurrent_jobs(),
            inherit_environment: false,
        }
    }
}

impl From<PythonLabToolConfig> for PythonLabConfig {
    fn from(value: PythonLabToolConfig) -> Self {
        Self {
            default_timeout_seconds: value.default_timeout_seconds,
            max_foreground_seconds: value.max_foreground_seconds,
            max_background_seconds: value.max_background_seconds,
            default_max_output_bytes: value.default_max_output_bytes,
            max_output_bytes: value.max_output_bytes,
            max_memory_bytes: value.max_memory_bytes,
            max_cpu_seconds: value.max_cpu_seconds,
            max_processes: value.max_processes,
            max_concurrent_jobs: value.max_concurrent_jobs,
            inherit_environment: value.inherit_environment,
        }
    }
}

fn default_python_lab_timeout_seconds() -> u64 {
    60
}
fn default_python_lab_max_foreground_seconds() -> u64 {
    300
}
fn default_python_lab_max_background_seconds() -> u64 {
    1800
}
fn default_python_lab_output_bytes() -> usize {
    200_000
}
fn default_python_lab_max_output_bytes() -> usize {
    1_000_000
}
fn default_python_lab_max_processes() -> Option<u32> {
    Some(1)
}
fn default_python_lab_concurrent_jobs() -> usize {
    2
}

impl Default for PythonLabConfig {
    fn default() -> Self {
        Self {
            default_timeout_seconds: 60,
            max_foreground_seconds: 300,
            max_background_seconds: 1800,
            default_max_output_bytes: 200_000,
            max_output_bytes: 1_000_000,
            max_memory_bytes: None,
            max_cpu_seconds: None,
            max_processes: Some(1),
            max_concurrent_jobs: 2,
            inherit_environment: false,
        }
    }
}

pub struct PythonLabTool {
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
    config: PythonLabConfig,
    session_key: std::sync::Mutex<String>,
}

impl PythonLabTool {
    pub fn new(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>, config: PythonLabConfig) -> Self {
        Self {
            workspace,
            sandbox,
            config,
            session_key: std::sync::Mutex::new(String::new()),
        }
    }
}

impl Tool for PythonLabTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "python_lab".into(),
            description: "Execute Python programs in the persistent task workspace for domain-neutral computation. Provide op=run with exactly one of code or path; programs may read and write workspace files. Prefer concise output and store large results as artifacts. Example: {\"op\":\"run\",\"code\":\"print(2 + 2)\"}".into(),
            parameters_schema: r#"{"type":"object","properties":{"op":{"type":"string","enum":["run","status","output","cancel"],"default":"run"},"code":{"type":"string"},"path":{"type":"string"},"args":{"type":"array","items":{"type":"string"}},"stdin":{"type":"string"},"timeout_seconds":{"type":"number"},"max_output_bytes":{"type":"number"},"background":{"type":"boolean"},"job_id":{"type":"string"},"offset":{"type":"number"},"limit":{"type":"number"}},"required":["op"]}"#.into(),
        }
    }

    fn set_session_key(&self, session_key: String) {
        if let Ok(mut g) = self.session_key.lock() {
            *g = session_key;
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(arguments);
        let workspace = self.workspace.clone();
        let sandbox = self.sandbox.clone();
        let cfg = self.config.clone();
        let session_key = self
            .session_key
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        Box::pin(async move {
            let v = match parsed {
                Ok(v) => v,
                Err(e) => return tool_err(format!("invalid JSON arguments: {e}")),
            };
            let op = v.get("op").and_then(|x| x.as_str()).unwrap_or("run");
            if op != "run" {
                return ok_json(
                    json!({"status":"not_implemented","op":op,"message":"python_lab background job operations are reserved for the background-jobs slice"}),
                    true,
                );
            }
            if v.get("background")
                .and_then(|x| x.as_bool())
                .unwrap_or(false)
            {
                return ok_json(
                    json!({"status":"not_implemented","message":"background execution is reserved for the background-jobs slice"}),
                    true,
                );
            }
            let code = v.get("code").and_then(|x| x.as_str());
            let path = v.get("path").and_then(|x| x.as_str());
            if code.is_some() == path.is_some() {
                return tool_err("exactly one of 'code' or 'path' is required".to_string());
            }
            let args: Vec<String> = match v.get("args") {
                None => Vec::new(),
                Some(value) => match value.as_array() {
                    Some(items) => {
                        let mut parsed = Vec::with_capacity(items.len());
                        for item in items {
                            let Some(arg) = item.as_str() else {
                                return tool_err("args must be an array of strings".to_string());
                            };
                            parsed.push(arg.to_string());
                        }
                        parsed
                    }
                    None => return tool_err("args must be an array of strings".to_string()),
                },
            };
            let stdin = v
                .get("stdin")
                .and_then(|x| x.as_str())
                .map(ToString::to_string);
            let timeout_secs = match bounded_u64(
                &v,
                "timeout_seconds",
                cfg.default_timeout_seconds,
                cfg.max_foreground_seconds,
            ) {
                Ok(n) => n,
                Err(msg) => return tool_err(msg),
            };
            let max_out = match bounded_u64(
                &v,
                "max_output_bytes",
                cfg.default_max_output_bytes as u64,
                cfg.max_output_bytes as u64,
            ) {
                Ok(n) => n as usize,
                Err(msg) => return tool_err(msg),
            };
            let exec_id = format!(
                "py_{}_{:x}_{}",
                std::process::id(),
                now_ms(),
                EXEC_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            );
            let start_ms = now_ms();
            let before = snapshot_files(&workspace);

            let mut cmd = Command::new("python3");
            cmd.current_dir(&*workspace).kill_on_drop(true);
            if !cfg.inherit_environment {
                cmd.env_clear()
                    .env(
                        "PATH",
                        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into()),
                    )
                    .env("PYTHONNOUSERSITE", "1");
            }
            let invocation_type;
            if let Some(src) = code {
                invocation_type = "inline";
                cmd.arg("-c").arg(src);
            } else {
                invocation_type = "file";
                let p = workspace.join(path.unwrap());
                let validated = sandbox
                    .validate_path(&p.to_string_lossy())
                    .map_err(|e| DomainError::Security(e.to_string()))?;
                cmd.arg("--").arg(validated);
            }
            for a in args {
                cmd.arg(a);
            }
            if stdin.is_some() {
                cmd.stdin(std::process::Stdio::piped());
            }
            let artifact_dir = workspace.join(".quecto/python_lab").join(&exec_id);
            tokio::fs::create_dir_all(&artifact_dir)
                .await
                .map_err(|e| DomainError::Other(e.to_string()))?;
            let stdout_path = artifact_dir.join("stdout.txt");
            let stderr_path = artifact_dir.join("stderr.txt");
            let stdout_file = std::fs::File::create(&stdout_path)
                .map_err(|e| DomainError::Other(e.to_string()))?;
            let stderr_file = std::fs::File::create(&stderr_path)
                .map_err(|e| DomainError::Other(e.to_string()))?;
            cmd.stdout(std::process::Stdio::from(stdout_file))
                .stderr(std::process::Stdio::from(stderr_file));
            let mut child = cmd
                .spawn()
                .map_err(|e| DomainError::Other(format!("failed to start python3: {e}")))?;
            if let Some(input) = stdin {
                if let Some(mut pipe) = child.stdin.take() {
                    tokio::spawn(async move {
                        let _ = pipe.write_all(input.as_bytes()).await;
                    });
                }
            }
            let timed = tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait()).await;
            let (status, exit_code) = match timed {
                Ok(Ok(status)) => ("completed", status.code()),
                Ok(Err(e)) => {
                    return Err(DomainError::Other(format!("python3 execution failed: {e}")));
                }
                Err(_) => {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    ("timed_out", None)
                }
            };
            let end_ms = now_ms();
            let (stdout_preview, stdout_trunc) = read_preview(&stdout_path, max_out).await?;
            let (stderr_preview, stderr_trunc) = read_preview(&stderr_path, max_out).await?;
            let mut artifact_paths = Vec::new();
            if stdout_trunc {
                artifact_paths.push(
                    stdout_path
                        .strip_prefix(&*workspace)
                        .unwrap()
                        .to_string_lossy()
                        .to_string(),
                );
            }
            if stderr_trunc {
                artifact_paths.push(
                    stderr_path
                        .strip_prefix(&*workspace)
                        .unwrap()
                        .to_string_lossy()
                        .to_string(),
                );
            }
            if artifact_paths.is_empty() {
                let _ = tokio::fs::remove_dir_all(&artifact_dir).await;
            }
            let changed = changed_files(&workspace, before);
            let result = json!({
                "status": status, "exit_code": exit_code, "execution_id": exec_id, "session_id": session_key,
                "invocation_type": invocation_type, "interpreter": "python3", "start_time_ms": start_ms, "completion_time_ms": end_ms,
                "duration_ms": end_ms.saturating_sub(start_ms), "timeout_seconds": timeout_secs, "stdout": stdout_preview, "stderr": stderr_preview,
                "output_truncated": stdout_trunc || stderr_trunc, "stdout_truncated": stdout_trunc, "stderr_truncated": stderr_trunc,
                "artifact_paths": artifact_paths, "files_created_or_modified": changed,
                "resource_limits": {"memory_bytes": cfg.max_memory_bytes, "cpu_seconds": cfg.max_cpu_seconds, "processes": cfg.max_processes}
            });
            ok_json(result, status != "completed" || exit_code.unwrap_or(0) != 0)
        })
    }
}

fn tool_err(content: String) -> Result<ToolResult, DomainError> {
    Ok(ToolResult {
        content,
        is_error: true,
        image_blocks: vec![],
    })
}
fn ok_json(v: serde_json::Value, is_error: bool) -> Result<ToolResult, DomainError> {
    Ok(ToolResult {
        content: serde_json::to_string_pretty(&v).unwrap(),
        is_error,
        image_blocks: vec![],
    })
}
static EXEC_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
fn snapshot_files(root: &std::path::Path) -> std::collections::BTreeMap<String, SystemTime> {
    let mut m = std::collections::BTreeMap::new();
    snapshot_rec(root, root, &mut m);
    m
}
fn snapshot_rec(
    root: &std::path::Path,
    dir: &std::path::Path,
    m: &mut std::collections::BTreeMap<String, SystemTime>,
) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                snapshot_rec(root, &p, m)
            } else if let Ok(md) = e.metadata() {
                if let Ok(rel) = p.strip_prefix(root) {
                    m.insert(
                        rel.to_string_lossy().to_string(),
                        md.modified().unwrap_or(UNIX_EPOCH),
                    );
                }
            }
        }
    }
}
fn changed_files(
    root: &std::path::Path,
    before: std::collections::BTreeMap<String, SystemTime>,
) -> Vec<String> {
    snapshot_files(root)
        .into_iter()
        .filter(|(p, t)| before.get(p).map(|b| b < t).unwrap_or(true))
        .map(|(p, _)| p)
        .collect()
}
async fn read_preview(path: &std::path::Path, max: usize) -> Result<(String, bool), DomainError> {
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|e| DomainError::Other(e.to_string()))?;
    let truncated = metadata.len() as usize > max;
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| DomainError::Other(e.to_string()))?;
    let mut buf = vec![0; max];
    let n = file
        .read(&mut buf)
        .await
        .map_err(|e| DomainError::Other(e.to_string()))?;
    buf.truncate(n);
    Ok((String::from_utf8_lossy(&buf).to_string(), truncated))
}

fn bounded_u64(
    value: &serde_json::Value,
    key: &str,
    default: u64,
    maximum: u64,
) -> Result<u64, String> {
    match value.get(key) {
        None => Ok(default.min(maximum)),
        Some(v) => v
            .as_u64()
            .map(|n| n.min(maximum))
            .ok_or_else(|| format!("{key} must be a non-negative integer")),
    }
}
