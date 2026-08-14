use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_json::json;

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

/// Registry of background jobs, keyed by job id.
type JobRegistry = Arc<Mutex<HashMap<String, Arc<Mutex<JobState>>>>>;

pub struct PythonLabTool {
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
    config: PythonLabConfig,
    session_key: Mutex<String>,
    jobs: JobRegistry,
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
                    // Terminal jobs have already been reaped. Signalling them
                    // would both stall teardown (each kill forks pgrep) and
                    // risk hitting a recycled pid.
                    if is_terminal(&j.status) {
                        continue;
                    }
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
    prune_artifact_dirs(&workspace, &jobs);
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
            evict_finished_jobs(&mut registry);
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
            // The terminal status is computed here but published only once the
            // result JSON is built. Flipping status first let a caller observe
            // "completed" while `result` was still null.
            let (canceled, max_output_bytes) = state
                .lock()
                .map(|s| (s.cancel_requested, s.max_output_bytes))
                .unwrap_or((false, spec_bg.max_out));
            let (st, exit_code) = match result {
                Ok((st, code)) => (st, code),
                Err(e) => (format!("failed: {e}"), None),
            };
            let final_status = if canceled {
                "cancelled".to_string()
            } else {
                st
            };
            let completed_ms = now_ms();
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
                s.exit_code = exit_code;
                s.completed_ms = Some(completed_ms);
                s.result = Some(res);
                // Published last: callers poll on status, so everything they
                // will read next must already be in place.
                s.status = final_status;
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
        // The tool's own artifact tree is off limits as a script source, so a
        // program cannot stage code inside another execution's directory.
        if is_reserved_artifact_rel(Path::new(p)) {
            return Err(DomainError::Security(
                ".quecto/python_lab is reserved for python_lab artifacts".into(),
            ));
        }
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
        // bounded_u64 already clamps max_out to cfg.max_output_bytes, so the
        // configured maximum is always the larger of the two.
        artifact_max_bytes: cfg.max_output_bytes,
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

async fn status_op(v: &serde_json::Value, jobs: JobRegistry) -> Result<ToolResult, DomainError> {
    let id = job_id(v)?;
    let Some(job) = jobs.lock().unwrap().get(id).cloned() else {
        return ok_json(json!({"status":"not_found","job_id":id}), true);
    };
    let s = job.lock().unwrap();
    ok_json(
        json!({"status":s.status,"job_id":id,"execution_id":s.execution_id,"session_id":s.session_id,"invocation_type":s.invocation_type,"exit_code":s.exit_code,"start_time_ms":s.started_ms,"completion_time_ms":s.completed_ms,"duration_ms":s.completed_ms.unwrap_or_else(now_ms).saturating_sub(s.started_ms),"timeout_seconds":s.timeout_seconds,"timeout_or_cancel_reason": if s.status=="timed_out" {"timeout"} else if s.status=="cancelled" || s.status=="cancelling" {"cancelled"} else {""},"resource_limits":s.resource_limits}),
        // The missing-job case already returned above, so a reported status is
        // never an error here.
        false,
    )
}
async fn output_op(
    v: &serde_json::Value,
    workspace: Arc<PathBuf>,
    jobs: JobRegistry,
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
    // Paging reads the artifacts back off disk so callers can walk output far
    // larger than the inline preview. Nothing stops a later program from
    // rewriting those files, so the sizes captured at completion are compared
    // against what is on disk now and any divergence is surfaced.
    let artifacts_modified = artifacts_diverged(result.as_ref(), &outp, &errp).await;
    let is_err = (status != "running" && status != "cancelling" && status != "completed")
        || (status == "completed" && exit_code.unwrap_or(0) != 0);
    ok_json(
        json!({"status":status,"job_id":id,"stdout":stdout.0,"stderr":stderr.0,"offset":offset,"limit":limit,"stdout_more":stdout.1,"stderr_more":stderr.1,"result":result,"artifacts_modified":artifacts_modified,"artifact_paths":[rel(&workspace,&outp),rel(&workspace,&errp)]}),
        is_err,
    )
}
/// True when an artifact's size no longer matches what was captured when the
/// job completed, which means something rewrote it after the fact. Reports
/// false while the job is still running and has no captured sizes yet.
async fn artifacts_diverged(
    result: Option<&serde_json::Value>,
    stdout_path: &Path,
    stderr_path: &Path,
) -> bool {
    let Some(usage) = result.and_then(|r| r.get("resource_usage")) else {
        return false;
    };
    for (key, path) in [
        ("stdout_bytes_retained", stdout_path),
        ("stderr_bytes_retained", stderr_path),
    ] {
        let Some(captured) = usage.get(key).and_then(|v| v.as_u64()) else {
            return false;
        };
        if file_len(path).await != Some(captured) {
            return true;
        }
    }
    false
}

async fn cancel_op(v: &serde_json::Value, jobs: JobRegistry) -> Result<ToolResult, DomainError> {
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
/// Executions whose artifact directories are kept on disk. Every run writes
/// stdout/stderr artifacts and nothing else ever deletes them, so without this
/// a long session grows the workspace by up to `max_output_bytes` per call,
/// forever.
const MAX_RETAINED_ARTIFACT_DIRS: usize = 32;

/// Deletes the oldest artifact directories once the retention ceiling is
/// passed. Directories belonging to a job that has not finished are never
/// removed, so a running program cannot have its output deleted underneath it.
fn prune_artifact_dirs(workspace: &Path, jobs: &JobRegistry) {
    let root = workspace.join(".quecto/python_lab");
    let live: Vec<String> = jobs
        .lock()
        .map(|registry| {
            registry
                .values()
                .filter_map(|job| {
                    let j = job.lock().ok()?;
                    (!is_terminal(&j.status)).then(|| j.execution_id.clone())
                })
                .collect()
        })
        .unwrap_or_default();
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    let mut dirs: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| !live.iter().any(|id| *id == e.file_name().to_string_lossy()))
        .filter_map(|e| {
            let modified = e.metadata().ok()?.modified().ok()?;
            Some((modified, e.path()))
        })
        .collect();
    if dirs.len() <= MAX_RETAINED_ARTIFACT_DIRS {
        return;
    }
    dirs.sort_unstable_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    for (_, path) in dirs.into_iter().skip(MAX_RETAINED_ARTIFACT_DIRS) {
        let _ = std::fs::remove_dir_all(path);
    }
}

/// A job is terminal once it has been reaped and its result published.
fn is_terminal(status: &str) -> bool {
    !matches!(status, "running" | "cancelling")
}

/// Completed jobs are kept so their results stay retrievable, but not forever:
/// each retained job holds its full result JSON. Once the retention ceiling is
/// reached the oldest finished jobs are dropped. Live jobs are never evicted.
const MAX_RETAINED_JOBS: usize = 32;

fn evict_finished_jobs(registry: &mut HashMap<String, Arc<Mutex<JobState>>>) {
    if registry.len() < MAX_RETAINED_JOBS {
        return;
    }
    let mut finished: Vec<(u128, String)> = registry
        .iter()
        .filter_map(|(id, job)| {
            let j = job.lock().ok()?;
            is_terminal(&j.status).then(|| (j.completed_ms.unwrap_or(j.started_ms), id.clone()))
        })
        .collect();
    finished.sort_unstable();
    for (_, id) in finished
        .into_iter()
        .take((registry.len() + 1).saturating_sub(MAX_RETAINED_JOBS))
    {
        registry.remove(&id);
    }
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
    // Byte counts are what was RETAINED after the cap clipped the streams, not
    // what the program produced — named accordingly so they are not read as
    // output volume. A missing artifact reports null rather than 0, so "no file"
    // stays distinguishable from "wrote nothing". Wall-clock time is already
    // reported as `duration_ms` on the enclosing object and is not repeated.
    let resource_usage = json!({
        "stdout_bytes_retained": file_len(ctx.stdout_path).await,
        "stderr_bytes_retained": file_len(ctx.stderr_path).await,
        "cpu_time_ms": serde_json::Value::Null,
        "max_rss_bytes": serde_json::Value::Null,
    });
    Ok(
        json!({"status":ctx.status,"exit_code":ctx.exit_code,"execution_id":ctx.exec_id,"session_id":ctx.session_key,"invocation_type":ctx.invocation_type,"interpreter":"python3","interpreter_version":interpreter_version(),"start_time_ms":ctx.start,"completion_time_ms":ctx.end,"duration_ms":ctx.end.saturating_sub(ctx.start),"timeout_seconds":ctx.timeout,"timeout_or_cancel_reason": if ctx.status=="timed_out" {"timeout"} else if ctx.status=="cancelled" {"cancelled"} else {""},"stdout":stdout,"stderr":stderr,"output_truncated":st||et,"stdout_truncated":st,"stderr_truncated":et,"artifact_paths":artifact_paths,"files_created_or_modified":ctx.changed,"resource_limits":{"memory_bytes":ctx.cfg.max_memory_bytes,"cpu_seconds":ctx.cfg.max_cpu_seconds,"processes":ctx.cfg.max_processes},"resource_usage":resource_usage}),
    )
}
/// `None` when the artifact is absent or unreadable, so a missing file is not
/// reported as a zero-byte one.
async fn file_len(path: &Path) -> Option<u64> {
    tokio::fs::metadata(path).await.ok().map(|m| m.len())
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
#[path = "python_lab_support.rs"]
mod python_lab_support;
pub(crate) use python_lab_support::*;
