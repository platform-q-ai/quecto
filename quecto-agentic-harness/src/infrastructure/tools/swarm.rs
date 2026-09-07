use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use serde_json::json;

#[path = "swarm_process.rs"]
mod swarm_process;
#[path = "swarm_result.rs"]
mod swarm_result;
use swarm_process::{interpreter_version, kill_pid, kill_pid_tree_best_effort, run_child};
use swarm_result::{ResultContext, artifacts_diverged, build_result, file_len};

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;

pub use super::swarm_config::{SwarmConfig, SwarmToolConfig};

/// Registry of background jobs, keyed by job id.
type JobRegistry = Arc<Mutex<HashMap<String, Arc<Mutex<JobState>>>>>;

/// Execution ids with a run in flight. Foreground runs never enter the job
/// registry, so without this their artifact directory is prunable from the
/// moment it is created — pruning it mid-run destroys the output the run is
/// about to read back.
type ActiveExecutions = Arc<Mutex<std::collections::HashSet<String>>>;

/// Removes its execution id on drop, so an early return or an error cannot
/// leave an id marked active forever.
struct ActiveGuard(ActiveExecutions, String);

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = self.0.lock() {
            set.remove(&self.1);
        }
    }
}

pub struct SwarmTool {
    context: Option<super::swarm_bridge::SwarmContext>,
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
    config: SwarmConfig,
    session_key: Mutex<String>,
    jobs: JobRegistry,
    active: ActiveExecutions,
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
    pub(crate) inherit_environment: bool,
}

impl SwarmTool {
    pub fn new(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>, config: SwarmConfig) -> Self {
        Self {
            context: None,
            workspace,
            sandbox,
            config,
            session_key: Mutex::new(String::new()),
            jobs: Arc::new(Mutex::new(HashMap::new())),
            active: Arc::new(Mutex::new(std::collections::HashSet::new())),
        }
    }
}

impl SwarmTool {
    pub fn with_context(mut self, context: Option<super::swarm_bridge::SwarmContext>) -> Self {
        self.context = context;
        self
    }
}

impl Drop for SwarmTool {
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

impl Tool for SwarmTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "swarm".into(),
            description: include_str!("swarm_helpers/tool_description.txt").into(),
            parameters_schema: include_str!("swarm_helpers/tool_schema.json").into(),
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
        let context = self.context.clone();
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(arguments);
        let workspace = self.workspace.clone();
        let sandbox = self.sandbox.clone();
        let cfg = self.config.clone();
        let jobs = self.jobs.clone();
        let active = self.active.clone();
        let session_key = self
            .session_key
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        Box::pin(async move {
            let Some(context) = context else {
                return tool_err("swarm is container-only: use spawn with a registered isolated container, then create a bounded run inside it".into());
            };
            let v = match parsed {
                Ok(v) => v,
                Err(e) => return tool_err(format!("invalid JSON arguments: {e}")),
            };
            match v.get("op").and_then(|x| x.as_str()).unwrap_or("run") {
                op @ ("create" | "summary" | "reconcile" | "cancel_run") => {
                    match super::swarm_control::control(context, op, v.clone()).await {
                        Ok(value) => {
                            if value["status"] == "cancelled" {
                                cancel_jobs(&jobs);
                            }
                            ok_json(value, false)
                        }
                        Err(error) => tool_err(error.to_string()),
                    }
                }
                "run" => {
                    run_op(
                        v,
                        RunEnv {
                            context,
                            workspace,
                            sandbox,
                            cfg,
                            jobs,
                            active,
                            session_key,
                        },
                    )
                    .await
                }
                "status" => status_op(&v, workspace, jobs).await,
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

/// Everything a run needs from the tool instance.
struct RunEnv {
    context: super::swarm_bridge::SwarmContext,
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
    cfg: SwarmConfig,
    jobs: JobRegistry,
    active: ActiveExecutions,
    session_key: String,
}

async fn run_op(v: serde_json::Value, env: RunEnv) -> Result<ToolResult, DomainError> {
    let RunEnv {
        context,
        workspace,
        sandbox,
        cfg,
        jobs,
        active,
        session_key,
    } = env;
    let summary = match super::swarm_control::execution_state(context.clone()).await {
        Ok(summary) => summary,
        Err(error) => return tool_err(error.to_string()),
    };
    if summary["status"] != "running" {
        return tool_err(format!("swarm is {}; inspect summary", summary["status"]));
    }
    let mut spec = match parse_run(&v, &workspace, &sandbox, &cfg) {
        Ok(s) => s,
        Err(e) => return tool_err(e.to_string()),
    };
    let remaining = summary["deadline"].as_f64().unwrap_or(0.0) - now_ms() as f64 / 1000.0;
    if remaining <= 0.0 {
        return tool_err("swarm budget-exhausted".into());
    }
    spec.timeout_secs = spec.timeout_secs.min(remaining.ceil() as u64);
    spec.bootstrap = Some(context.bootstrap());
    let exec_id = format!(
        "py_{}_{:x}_{}",
        std::process::id(),
        now_ms(),
        EXEC_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let start = now_ms();
    let before = {
        let ws = workspace.clone();
        // Recursive stat of the whole workspace: blocking work, not async work.
        tokio::task::spawn_blocking(move || snapshot_files(&ws))
            .await
            .map_err(|e| DomainError::Other(e.to_string()))?
    };
    let artifact_dir = workspace.join(".quecto/swarm").join(&exec_id);
    tokio::fs::create_dir_all(&artifact_dir)
        .await
        .map_err(ioerr)?;
    if let Ok(mut set) = active.lock() {
        set.insert(exec_id.clone());
    }
    // Held for the rest of the call. A background run also registers a job, so
    // it stays protected after this guard drops at the end of run_op.
    let _active_guard = ActiveGuard(active.clone(), exec_id.clone());
    {
        let (ws, jobs, active) = (workspace.clone(), jobs.clone(), active.clone());
        // read_dir plus an unbounded number of remove_dir_all calls must not
        // run on the async worker thread.
        let _ = tokio::task::spawn_blocking(move || prune_artifact_dirs(&ws, &jobs, &active)).await;
    }
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
            inherit_environment: cfg.inherit_environment,
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
                    json!({"status":"rejected","message":"swarm concurrent job limit reached","max_concurrent_jobs":cfg.max_concurrent_jobs}),
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
            let changed = {
                let ws = workspace.clone();
                tokio::task::spawn_blocking(move || changed_files(&ws, before))
                    .await
                    .unwrap_or_default()
            };
            let res = build_result(ResultContext {
                status: &final_status,
                exit_code,
                exec_id: &exec_id_bg,
                _session_key: &session_key_bg,
                _invocation_type: &spec_bg.invocation_type,
                background: true,
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
            let mut res = res;
            match super::swarm_control::after_execution(context, &summary).await {
                Ok(warnings) if !warnings.is_empty() => {
                    res["notification_warnings"] = json!(warnings)
                }
                Err(error) => res["coordination_error"] = json!(error.to_string()),
                _ => {}
            }
            if let Ok(mut s) = state.lock() {
                s.exit_code = exit_code;
                s.completed_ms = Some(completed_ms);
                s.result = Some(res);
                // Re-read the cancel flag under the publishing lock. Building
                // the result reads both artifacts, and a cancel arriving during
                // that window would otherwise be overwritten — the caller would
                // be told "cancelling" and the job would report "completed".
                s.status = if s.cancel_requested {
                    "cancelled".to_string()
                } else {
                    final_status
                };
            }
        });
        return ok_json(
            json!({"status":"running","job_id":job_id,"execution_id":exec_id}),
            false,
        );
    }
    let (status, code) =
        run_child(spec.clone(), &workspace, &stdout_path, &stderr_path, None).await?;
    let end = now_ms();
    let changed = {
        let ws = workspace.clone();
        tokio::task::spawn_blocking(move || changed_files(&ws, before))
            .await
            .map_err(|e| DomainError::Other(e.to_string()))?
    };
    let result = build_result(ResultContext {
        status: &status,
        exit_code: code,
        exec_id: &exec_id,
        _session_key: &session_key,
        _invocation_type: &spec.invocation_type,
        background: false,
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
    let warnings = super::swarm_control::after_execution(context, &summary).await?;
    let mut result = result;
    if !warnings.is_empty() {
        result["notification_warnings"] = json!(warnings);
    }
    let is_err = status != "completed" || code.unwrap_or(0) != 0;
    ok_json(result, is_err)
}

#[derive(Clone)]
pub(crate) struct RunSpec {
    pub(crate) bootstrap: Option<String>,
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
    cfg: &SwarmConfig,
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
        if is_reserved_artifact_path(workspace, Path::new(p)) {
            return Err(DomainError::Security(
                ".quecto/swarm is reserved for swarm artifacts".into(),
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
        bootstrap: None,
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

async fn status_op(
    v: &serde_json::Value,
    workspace: Arc<PathBuf>,
    jobs: JobRegistry,
) -> Result<ToolResult, DomainError> {
    let id = job_id(v)?;
    let Some(job) = jobs.lock().unwrap().get(id).cloned() else {
        return ok_json(json!({"status":"not_found","job_id":id}), true);
    };
    let (
        status,
        execution_id,
        session_id,
        invocation_type,
        exit_code,
        started_ms,
        completed_ms,
        timeout_seconds,
        resource_limits,
        inherit_environment,
        stdout_path,
        stderr_path,
        max_output_bytes,
        terminal_result,
    ) = {
        let s = job.lock().unwrap();
        (
            s.status.clone(),
            s.execution_id.clone(),
            s.session_id.clone(),
            s.invocation_type.clone(),
            s.exit_code,
            s.started_ms,
            s.completed_ms,
            s.timeout_seconds,
            s.resource_limits.clone(),
            s.inherit_environment,
            s.stdout_path.clone(),
            s.stderr_path.clone(),
            s.max_output_bytes,
            s.result.clone(),
        )
    };
    let mut detail = json!({"status":status,"job_id":id,"execution_id":execution_id,"session_id":session_id,"invocation_type":invocation_type,"interpreter":"python3","interpreter_version":interpreter_version(inherit_environment),"exit_code":exit_code,"start_time_ms":started_ms,"completion_time_ms":completed_ms,"duration_ms":completed_ms.unwrap_or_else(now_ms).saturating_sub(started_ms),"timeout_seconds":timeout_seconds,"timeout_or_cancel_reason": if status=="timed_out" {"timeout"} else if status=="cancelled" || status=="cancelling" {"cancelled"} else {""},"resource_limits":resource_limits,"resource_usage":{"stdout_bytes_retained":file_len(&stdout_path).await,"stderr_bytes_retained":file_len(&stderr_path).await,"cpu_time_ms":serde_json::Value::Null,"max_rss_bytes":serde_json::Value::Null}});
    if let Some(result) = terminal_result {
        if let Some(obj) = detail.as_object_mut() {
            obj.insert("resource_usage".into(), json!({"stdout_bytes_retained":file_len(&stdout_path).await,"stderr_bytes_retained":file_len(&stderr_path).await,"cpu_time_ms":serde_json::Value::Null,"max_rss_bytes":serde_json::Value::Null}));
            if let Some(files) = result.get("files_created_or_modified") {
                obj.insert("files_created_or_modified".into(), files.clone());
            }
            if let Some(paths) = result.get("artifact_paths") {
                obj.insert("artifact_paths".into(), paths.clone());
            }
            obj.entry("files_created_or_modified")
                .or_insert_with(|| json!([]));
            for key in ["output_truncated", "stdout_truncated", "stderr_truncated"] {
                if let Some(value) = result.get(key) {
                    obj.insert(key.into(), value.clone());
                }
            }
        }
    } else {
        let artifacts = [stdout_path.as_path(), stderr_path.as_path()]
            .into_iter()
            .filter(|p| p.exists())
            .map(artifact_rel)
            .collect::<Vec<_>>();
        if let Some(obj) = detail.as_object_mut() {
            obj.insert("artifact_paths".into(), json!(artifacts));
        }
    }
    if let Some(obj) = detail.as_object_mut() {
        let artifact_root = workspace.join(format!(".quecto/swarm/{execution_id}"));
        obj.insert("artifact_dir".into(), json!(artifact_rel(&artifact_root)));
        obj.insert("max_output_bytes".into(), json!(max_output_bytes));
    }
    ok_json(
        detail,
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

fn job_id(v: &serde_json::Value) -> Result<&str, DomainError> {
    v.get("job_id")
        .and_then(|x| x.as_str())
        .ok_or_else(|| DomainError::Other("job_id is required".into()))
}

fn tool_err(content: String) -> Result<ToolResult, DomainError> {
    Ok(ToolResult {
        content,
        is_error: true,
        image_blocks: vec![],
        delivery_metadata: None,
    })
}
fn ok_json(v: serde_json::Value, is_error: bool) -> Result<ToolResult, DomainError> {
    Ok(ToolResult {
        content: serde_json::to_string_pretty(&v).unwrap(),
        is_error,
        image_blocks: vec![],
        delivery_metadata: None,
    })
}
#[path = "swarm_support.rs"]
mod swarm_support;
pub(crate) use swarm_support::*;
#[path = "swarm_registry.rs"]
mod swarm_registry;
pub(crate) use swarm_registry::*;

fn cancel_jobs(jobs: &JobRegistry) {
    if let Ok(jobs) = jobs.lock() {
        for job in jobs.values() {
            if let Ok(mut job) = job.lock() {
                if !is_terminal(&job.status) {
                    job.cancel_requested = true;
                    if let Some(pid) = job.pid {
                        terminate_member(pid);
                    }
                }
            }
        }
    }
}

pub(crate) fn terminate_member(pid: u32) {
    // Snapshot/terminate descendants while the parent is still present, then
    // use the existing group-or-PID fallback for both local and script-managed joins.
    kill_pid_tree_best_effort(pid);
    kill_pid(pid);
}
