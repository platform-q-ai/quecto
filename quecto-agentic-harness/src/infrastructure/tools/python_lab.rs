use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::AsyncReadExt;

#[path = "python_lab_process.rs"]
mod python_lab_process;
use python_lab_process::{interpreter_version, kill_pid, kill_pid_tree_best_effort, run_child};

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
    fn from(v: PythonLabToolConfig) -> Self {
        Self {
            default_timeout_seconds: v.default_timeout_seconds,
            max_foreground_seconds: v.max_foreground_seconds,
            max_background_seconds: v.max_background_seconds,
            default_max_output_bytes: v.default_max_output_bytes,
            max_output_bytes: v.max_output_bytes,
            max_memory_bytes: v.max_memory_bytes,
            max_cpu_seconds: v.max_cpu_seconds,
            max_processes: v.max_processes,
            max_concurrent_jobs: v.max_concurrent_jobs,
            inherit_environment: v.inherit_environment,
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
        PythonLabToolConfig::default().into()
    }
}

pub struct PythonLabTool {
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
    config: PythonLabConfig,
    session_key: Mutex<String>,
    jobs: Arc<Mutex<HashMap<String, Arc<Mutex<JobState>>>>>,
}

#[derive(Debug)]
pub(crate) struct JobState {
    pub(crate) execution_id: String,
    pub(crate) status: String,
    pub(crate) exit_code: Option<i32>,
    pub(crate) pid: Option<u32>,
    pub(crate) started_ms: u128,
    pub(crate) completed_ms: Option<u128>,
    pub(crate) stdout_path: PathBuf,
    pub(crate) stderr_path: PathBuf,
    pub(crate) max_output_bytes: usize,
    pub(crate) result: Option<serde_json::Value>,
    pub(crate) cancel_requested: bool,
    pub(crate) session_id: String,
    pub(crate) invocation_type: String,
    pub(crate) timeout_seconds: u64,
    pub(crate) resource_limits: serde_json::Value,
}

impl PythonLabTool {
    pub fn new(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>, config: PythonLabConfig) -> Self {
        Self {
            workspace,
            sandbox,
            config,
            session_key: Mutex::new(String::new()),
            jobs: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Drop for PythonLabTool {
    fn drop(&mut self) {
        if let Ok(jobs) = self.jobs.lock() {
            for job in jobs.values() {
                if let Ok(mut j) = job.lock() {
                    j.cancel_requested = true;
                    if let Some(pid) = j.pid {
                        kill_pid(pid);
                        kill_pid_tree_best_effort(pid);
                    }
                }
            }
        }
    }
}

impl Tool for PythonLabTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition { name: "python_lab".into(), description: "Execute Python programs in the persistent task workspace for domain-neutral computation. Provide op=run with exactly one of code or path; programs may read and write workspace files. Use background=true for long computations, then status/output/cancel by job_id. Prefer concise output and store large results as artifacts. Example: {\"op\":\"run\",\"code\":\"print(2 + 2)\"}".into(), parameters_schema: r#"{"type":"object","properties":{"op":{"type":"string","enum":["run","status","output","cancel"],"default":"run"},"code":{"type":"string"},"path":{"type":"string"},"args":{"type":"array","items":{"type":"string"}},"stdin":{"type":"string"},"timeout_seconds":{"type":"number"},"max_output_bytes":{"type":"number"},"background":{"type":"boolean"},"job_id":{"type":"string"},"offset":{"type":"number"},"limit":{"type":"number"}},"required":["op"]}"#.into() }
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
        let jobs = self.jobs.clone();
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
            match v.get("op").and_then(|x| x.as_str()).unwrap_or("run") {
                "run" => run_op(v, workspace, sandbox, cfg, jobs, session_key).await,
                "status" => status_op(&v, jobs).await,
                "output" => output_op(&v, workspace, jobs).await,
                "cancel" => cancel_op(&v, jobs).await,
                op => ok_json(
                    json!({"status":"error","message":format!("unknown op {op}")}),
                    true,
                ),
            }
        })
    }
}

async fn run_op(
    v: serde_json::Value,
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
    cfg: PythonLabConfig,
    jobs: Arc<Mutex<HashMap<String, Arc<Mutex<JobState>>>>>,
    session_key: String,
) -> Result<ToolResult, DomainError> {
    let spec = match parse_run(&v, &workspace, &sandbox, &cfg) {
        Ok(s) => s,
        Err(e) => return tool_err(e.to_string()),
    };
    let exec_id = format!(
        "py_{}_{:x}_{}",
        std::process::id(),
        now_ms(),
        EXEC_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let start = now_ms();
    let before = snapshot_files(&workspace);
    let artifact_dir = workspace.join(".quecto/python_lab").join(&exec_id);
    tokio::fs::create_dir_all(&artifact_dir)
        .await
        .map_err(ioerr)?;
    let stdout_path = artifact_dir.join("stdout.txt");
    let stderr_path = artifact_dir.join("stderr.txt");
    if spec.background {
        let job_id = format!("job_{}", exec_id);
        let state = Arc::new(Mutex::new(JobState {
            execution_id: exec_id.clone(),
            status: "running".into(),
            exit_code: None,
            pid: None,
            started_ms: start,
            completed_ms: None,
            stdout_path: stdout_path.clone(),
            stderr_path: stderr_path.clone(),
            max_output_bytes: spec.max_out,
            result: None,
            cancel_requested: false,
            session_id: session_key.clone(),
            invocation_type: spec.invocation_type.clone(),
            timeout_seconds: spec.timeout_secs,
            resource_limits: json!({"memory_bytes":cfg.max_memory_bytes,"cpu_seconds":cfg.max_cpu_seconds,"processes":cfg.max_processes}),
        }));
        {
            let mut registry = jobs.lock().unwrap();
            let running = registry
                .values()
                .filter(|j| {
                    j.lock()
                        .map(|s| s.status == "running" || s.status == "cancelling")
                        .unwrap_or(false)
                })
                .count();
            if running >= cfg.max_concurrent_jobs {
                return ok_json(
                    json!({"status":"rejected","message":"python_lab concurrent job limit reached","max_concurrent_jobs":cfg.max_concurrent_jobs}),
                    true,
                );
            }
            registry.insert(job_id.clone(), state.clone());
        }
        let spec_bg = spec.clone();
        let exec_id_bg = exec_id.clone();
        let session_key_bg = session_key.clone();
        let cfg_bg = cfg.clone();
        tokio::spawn(async move {
            let result = run_child(
                spec_bg.clone(),
                &workspace,
                &stdout_path,
                &stderr_path,
                Some(state.clone()),
            )
            .await;
            let (final_status, exit_code, completed_ms, max_output_bytes) = {
                let mut s = state.lock().unwrap();
                let canceled = s.cancel_requested;
                let (st, code) = match result {
                    Ok((st, code)) => (st, code),
                    Err(e) => (format!("failed: {e}"), None),
                };
                s.status = if canceled { "cancelled".into() } else { st };
                s.exit_code = code;
                s.completed_ms = Some(now_ms());
                (
                    s.status.clone(),
                    s.exit_code,
                    s.completed_ms.unwrap(),
                    s.max_output_bytes,
                )
            };
            let changed = changed_files(&workspace, before);
            let res = build_result(ResultContext {
                status: &final_status,
                exit_code,
                exec_id: &exec_id_bg,
                session_key: &session_key_bg,
                invocation_type: &spec_bg.invocation_type,
                start,
                end: completed_ms,
                timeout: spec_bg.timeout_secs,
                stdout_path: &stdout_path,
                stderr_path: &stderr_path,
                max_out: max_output_bytes,
                changed,
                cfg: &cfg_bg,
            })
            .await
            .unwrap_or_else(|e| json!({"status":"failed","message":e.to_string()}));
            if let Ok(mut s) = state.lock() {
                s.result = Some(res);
            }
        });
        return ok_json(
            json!({"status":"running","job_id":job_id,"execution_id":exec_id,"session_id":session_key,"start_time_ms":start,"timeout_seconds":spec.timeout_secs}),
            false,
        );
    }
    let (status, code) =
        run_child(spec.clone(), &workspace, &stdout_path, &stderr_path, None).await?;
    let end = now_ms();
    let changed = changed_files(&workspace, before);
    let result = build_result(ResultContext {
        status: &status,
        exit_code: code,
        exec_id: &exec_id,
        session_key: &session_key,
        invocation_type: &spec.invocation_type,
        start,
        end,
        timeout: spec.timeout_secs,
        stdout_path: &stdout_path,
        stderr_path: &stderr_path,
        max_out: spec.max_out,
        changed,
        cfg: &cfg,
    })
    .await?;
    let is_err = status != "completed" || code.unwrap_or(0) != 0;
    ok_json(result, is_err)
}

#[derive(Clone)]
pub(crate) struct RunSpec {
    pub(crate) invocation_type: String,
    pub(crate) code: Option<String>,
    pub(crate) script: Option<PathBuf>,
    pub(crate) args: Vec<String>,
    pub(crate) stdin: Option<String>,
    pub(crate) timeout_secs: u64,
    /// Cap on the preview echoed back inline in the tool result.
    pub(crate) max_out: usize,
    /// Cap on what is written to the workspace artifact. Always the configured
    /// hard maximum rather than the per-call preview cap, so the full output
    /// stays recoverable after the inline preview is truncated.
    pub(crate) artifact_max_bytes: usize,
    pub(crate) background: bool,
    pub(crate) inherit_environment: bool,
    pub(crate) max_memory_bytes: Option<u64>,
    pub(crate) max_cpu_seconds: Option<u64>,
    pub(crate) max_processes: Option<u32>,
}
fn parse_run(
    v: &serde_json::Value,
    workspace: &Path,
    sandbox: &Sandbox,
    cfg: &PythonLabConfig,
) -> Result<RunSpec, DomainError> {
    let code = v.get("code").and_then(|x| x.as_str()).map(str::to_string);
    let path = v.get("path").and_then(|x| x.as_str());
    if code.is_some() == path.is_some() {
        return Err(DomainError::Other(
            "exactly one of 'code' or 'path' is required".into(),
        ));
    }
    let args = match v.get("args") {
        None => vec![],
        Some(a) => a
            .as_array()
            .ok_or_else(|| DomainError::Other("args must be an array of strings".into()))?
            .iter()
            .map(|i| {
                i.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| DomainError::Other("args must be an array of strings".into()))
            })
            .collect::<Result<Vec<_>, _>>()?,
    };
    let timeout_secs = bounded_u64(
        v,
        "timeout_seconds",
        cfg.default_timeout_seconds,
        if v.get("background")
            .and_then(|x| x.as_bool())
            .unwrap_or(false)
        {
            cfg.max_background_seconds
        } else {
            cfg.max_foreground_seconds
        },
    )
    .map_err(DomainError::Other)?;
    let max_out = bounded_u64(
        v,
        "max_output_bytes",
        cfg.default_max_output_bytes as u64,
        cfg.max_output_bytes as u64,
    )
    .map_err(DomainError::Other)? as usize;
    let script = if let Some(p) = path {
        let p = workspace.join(p);
        Some(
            sandbox
                .validate_path(&p.to_string_lossy())
                .map_err(|e| DomainError::Security(e.to_string()))?,
        )
    } else {
        None
    };
    Ok(RunSpec {
        invocation_type: if code.is_some() { "inline" } else { "file" }.into(),
        code,
        script,
        args,
        stdin: v.get("stdin").and_then(|x| x.as_str()).map(str::to_string),
        timeout_secs,
        max_out,
        artifact_max_bytes: cfg.max_output_bytes.max(max_out),
        background: v
            .get("background")
            .and_then(|x| x.as_bool())
            .unwrap_or(false),
        inherit_environment: cfg.inherit_environment,
        max_memory_bytes: cfg.max_memory_bytes,
        max_cpu_seconds: cfg.max_cpu_seconds,
        max_processes: cfg.max_processes,
    })
}

async fn status_op(
    v: &serde_json::Value,
    jobs: Arc<Mutex<HashMap<String, Arc<Mutex<JobState>>>>>,
) -> Result<ToolResult, DomainError> {
    let id = job_id(v)?;
    let Some(job) = jobs.lock().unwrap().get(id).cloned() else {
        return ok_json(json!({"status":"not_found","job_id":id}), true);
    };
    let s = job.lock().unwrap();
    ok_json(
        json!({"status":s.status,"job_id":id,"execution_id":s.execution_id,"session_id":s.session_id,"invocation_type":s.invocation_type,"exit_code":s.exit_code,"start_time_ms":s.started_ms,"completion_time_ms":s.completed_ms,"duration_ms":s.completed_ms.unwrap_or_else(now_ms).saturating_sub(s.started_ms),"timeout_seconds":s.timeout_seconds,"timeout_or_cancel_reason": if s.status=="timed_out" {"timeout"} else if s.status=="cancelled" || s.status=="cancelling" {"cancelled"} else {""},"resource_limits":s.resource_limits}),
        s.status == "not_found",
    )
}
async fn output_op(
    v: &serde_json::Value,
    workspace: Arc<PathBuf>,
    jobs: Arc<Mutex<HashMap<String, Arc<Mutex<JobState>>>>>,
) -> Result<ToolResult, DomainError> {
    let id = job_id(v)?;
    let offset = bounded_u64(v, "offset", 0, u64::MAX).map_err(DomainError::Other)? as usize;
    let limit = bounded_u64(v, "limit", 200_000, 1_000_000).map_err(DomainError::Other)? as usize;
    let Some(job) = jobs.lock().unwrap().get(id).cloned() else {
        return ok_json(json!({"status":"not_found","job_id":id}), true);
    };
    let (status, exit_code, outp, errp, result) = {
        let s = job.lock().unwrap();
        (
            s.status.clone(),
            s.exit_code,
            s.stdout_path.clone(),
            s.stderr_path.clone(),
            s.result.clone(),
        )
    };
    let stdout = read_slice(&outp, offset, limit).await?;
    let stderr = read_slice(&errp, offset, limit).await?;
    let is_err = (status != "running" && status != "cancelling" && status != "completed")
        || (status == "completed" && exit_code.unwrap_or(0) != 0);
    ok_json(
        json!({"status":status,"job_id":id,"stdout":stdout.0,"stderr":stderr.0,"offset":offset,"limit":limit,"stdout_more":stdout.1,"stderr_more":stderr.1,"result":result,"artifact_paths":[rel(&workspace,&outp),rel(&workspace,&errp)]}),
        is_err,
    )
}
async fn cancel_op(
    v: &serde_json::Value,
    jobs: Arc<Mutex<HashMap<String, Arc<Mutex<JobState>>>>>,
) -> Result<ToolResult, DomainError> {
    let id = job_id(v)?;
    let Some(job) = jobs.lock().unwrap().get(id).cloned() else {
        return ok_json(json!({"status":"not_found","job_id":id}), true);
    };
    let mut s = job.lock().unwrap();
    if s.status != "running" {
        return ok_json(
            json!({"status":s.status,"job_id":id,"execution_id":s.execution_id,"message":"job is already terminal"}),
            false,
        );
    }
    s.cancel_requested = true;
    if let Some(pid) = s.pid {
        kill_pid(pid);
        kill_pid_tree_best_effort(pid);
    }
    s.status = "cancelling".into();
    ok_json(
        json!({"status":"cancelling","job_id":id,"execution_id":s.execution_id}),
        false,
    )
}
fn job_id(v: &serde_json::Value) -> Result<&str, DomainError> {
    v.get("job_id")
        .and_then(|x| x.as_str())
        .ok_or_else(|| DomainError::Other("job_id is required".into()))
}

struct ResultContext<'a> {
    status: &'a str,
    exit_code: Option<i32>,
    exec_id: &'a str,
    session_key: &'a str,
    invocation_type: &'a str,
    start: u128,
    end: u128,
    timeout: u64,
    stdout_path: &'a Path,
    stderr_path: &'a Path,
    max_out: usize,
    changed: Vec<String>,
    cfg: &'a PythonLabConfig,
}

async fn build_result(ctx: ResultContext<'_>) -> Result<serde_json::Value, DomainError> {
    let (stdout, stdout_full) = read_preview(ctx.stdout_path, ctx.max_out).await?;
    let (stderr, stderr_full) = read_preview(ctx.stderr_path, ctx.max_out).await?;
    let stdout_marker = truncation_marker_exists(ctx.stdout_path).await?;
    let stderr_marker = truncation_marker_exists(ctx.stderr_path).await?;
    let st = stdout_full || stdout_marker;
    let et = stderr_full || stderr_marker;
    let artifact_paths = [(st, ctx.stdout_path), (et, ctx.stderr_path)]
        .into_iter()
        .filter(|(t, _)| *t)
        .map(|(_, p)| artifact_rel(p))
        .collect::<Vec<_>>();
    // Per-process CPU and peak-RSS accounting would need wait4(2) rusage, which
    // tokio::process reaps internally and does not expose. Those fields are
    // reported as null rather than omitted so consumers can tell "not measured"
    // apart from "measured as zero".
    let resource_usage = json!({
        "wall_clock_ms": ctx.end.saturating_sub(ctx.start),
        "stdout_bytes": file_len(ctx.stdout_path).await,
        "stderr_bytes": file_len(ctx.stderr_path).await,
        "cpu_time_ms": serde_json::Value::Null,
        "max_rss_bytes": serde_json::Value::Null,
    });
    Ok(
        json!({"status":ctx.status,"exit_code":ctx.exit_code,"execution_id":ctx.exec_id,"session_id":ctx.session_key,"invocation_type":ctx.invocation_type,"interpreter":"python3","interpreter_version":interpreter_version(),"start_time_ms":ctx.start,"completion_time_ms":ctx.end,"duration_ms":ctx.end.saturating_sub(ctx.start),"timeout_seconds":ctx.timeout,"timeout_or_cancel_reason": if ctx.status=="timed_out" {"timeout"} else if ctx.status=="cancelled" {"cancelled"} else {""},"stdout":stdout,"stderr":stderr,"output_truncated":st||et,"stdout_truncated":st,"stderr_truncated":et,"artifact_paths":artifact_paths,"files_created_or_modified":ctx.changed,"resource_limits":{"memory_bytes":ctx.cfg.max_memory_bytes,"cpu_seconds":ctx.cfg.max_cpu_seconds,"processes":ctx.cfg.max_processes},"resource_usage":resource_usage}),
    )
}
async fn file_len(path: &Path) -> u64 {
    tokio::fs::metadata(path)
        .await
        .map(|m| m.len())
        .unwrap_or(0)
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
fn snapshot_files(root: &Path) -> BTreeMap<String, SystemTime> {
    let mut m = BTreeMap::new();
    snapshot_rec(root, root, &mut m);
    m
}
fn snapshot_rec(root: &Path, dir: &Path, m: &mut BTreeMap<String, SystemTime>) {
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
fn changed_files(root: &Path, before: BTreeMap<String, SystemTime>) -> Vec<String> {
    snapshot_files(root)
        .into_iter()
        .filter(|(p, t)| before.get(p).map(|b| b < t).unwrap_or(true))
        .map(|(p, _)| p)
        .collect()
}
async fn truncation_marker_exists(path: &Path) -> Result<bool, DomainError> {
    let mut marker = path.to_path_buf();
    marker.set_extension("truncated");
    match tokio::fs::metadata(marker).await {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(ioerr(e)),
    }
}

async fn read_preview(path: &Path, max: usize) -> Result<(String, bool), DomainError> {
    let md = tokio::fs::metadata(path).await.map_err(ioerr)?;
    let trunc = md.len() as usize > max;
    let mut f = tokio::fs::File::open(path).await.map_err(ioerr)?;
    let mut buf = vec![0; max];
    let n = f.read(&mut buf).await.map_err(ioerr)?;
    buf.truncate(n);
    Ok((String::from_utf8_lossy(&buf).to_string(), trunc))
}
async fn read_slice(
    path: &Path,
    offset: usize,
    limit: usize,
) -> Result<(String, bool), DomainError> {
    use tokio::io::AsyncSeekExt;
    let metadata = match tokio::fs::metadata(path).await {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((String::new(), false)),
        Err(e) => return Err(ioerr(e)),
    };
    let len = metadata.len();
    if offset as u128 >= len as u128 {
        return Ok((String::new(), false));
    }
    let mut file = tokio::fs::File::open(path).await.map_err(ioerr)?;
    file.seek(std::io::SeekFrom::Start(offset as u64))
        .await
        .map_err(ioerr)?;
    let to_read = limit.min((len - offset as u64) as usize);
    let mut buf = vec![0; to_read];
    let n = file.read(&mut buf).await.map_err(ioerr)?;
    buf.truncate(n);
    Ok((
        String::from_utf8_lossy(&buf).to_string(),
        (offset as u64).saturating_add(n as u64) < len,
    ))
}
fn rel(workspace: &Path, p: &Path) -> String {
    p.strip_prefix(workspace)
        .unwrap_or(p)
        .to_string_lossy()
        .to_string()
}
fn artifact_rel(p: &Path) -> String {
    let parts: Vec<_> = p
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    if let Some(i) = parts.iter().position(|x| x == ".quecto") {
        parts[i..].join("/")
    } else {
        p.to_string_lossy().to_string()
    }
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
fn ioerr(e: std::io::Error) -> DomainError {
    DomainError::Other(e.to_string())
}
