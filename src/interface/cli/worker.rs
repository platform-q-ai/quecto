//! `quecto worker` subcommand — coding worker entrypoint.
//!
//! Parses CLI flags, validates the job directory, builds the worker
//! tool registry and event emitter, and runs the coding agent loop.
//! Designed to run inside nsjail with JSON Lines IPC.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::application::worker_loop::{WorkerLoopConfig, WorkerLoopParams, run_worker_loop};
use crate::domain::coding_ports::WorkerEventSink;
use crate::domain::provider::LlmProvider;
use crate::infrastructure::coding::worker_event_emitter::{
    EmitterConfig, WorkerEventEmitter, WorkerEventSinkAdapter,
};
use crate::infrastructure::coding::worker_tool_wrappers::build_worker_tool_registry;

// ── Parsed arguments ────────────────────────────────────────────────────

/// Parsed worker command-line arguments.
#[derive(Debug, Clone)]
pub struct WorkerArgs {
    pub run_id: String,
    pub job_id: String,
    pub job_dir: String,
    pub goal: String,
    pub model: Option<String>,
    pub max_iterations: Option<u32>,
}

/// Parse worker flags from a slice of CLI arguments.
pub fn parse_worker_args(args: &[String]) -> Result<WorkerArgs, String> {
    let mut run_id: Option<String> = None;
    let mut job_id: Option<String> = None;
    let mut job_dir: Option<String> = None;
    let mut goal: Option<String> = None;
    let mut model: Option<String> = None;
    let mut max_iterations: Option<u32> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--run-id" => {
                run_id = Some(require_next(args, &mut i, "--run-id")?);
            }
            "--job-id" => {
                job_id = Some(require_next(args, &mut i, "--job-id")?);
            }
            "--job-dir" => {
                job_dir = Some(require_next(args, &mut i, "--job-dir")?);
            }
            "--goal" => {
                goal = Some(require_next(args, &mut i, "--goal")?);
            }
            "--model" => {
                model = Some(require_next(args, &mut i, "--model")?);
            }
            "--max-iterations" => {
                let val = require_next(args, &mut i, "--max-iterations")?;
                let n: u32 = val
                    .parse()
                    .map_err(|_| format!("--max-iterations must be a number, got '{val}'"))?;
                max_iterations = Some(n);
            }
            "--help" | "-h" => {
                return Err("worker: see documentation for usage".to_string());
            }
            other if other.starts_with("--") || other.starts_with('-') => {
                return Err(format!("unknown flag '{other}'"));
            }
            _ => {
                i += 1;
                continue;
            }
        }
    }

    let run_id = run_id.ok_or("missing required flag --run-id")?;
    let job_id = job_id.ok_or("missing required flag --job-id")?;
    let job_dir = job_dir.ok_or("missing required flag --job-dir")?;
    let goal = goal.ok_or("missing required flag --goal")?;

    Ok(WorkerArgs {
        run_id,
        job_id,
        job_dir,
        goal,
        model,
        max_iterations,
    })
}

fn require_next(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    *i += 1;
    if *i < args.len() {
        let val = args[*i].clone();
        *i += 1;
        Ok(val)
    } else {
        Err(format!("{flag} requires a value"))
    }
}

// ── Job directory validation ────────────────────────────────────────────

/// Validate that the job directory exists, is a directory, and is safe.
///
/// Canonicalizes the path to resolve symlinks and `..` components,
/// then checks that it is an absolute path. Returns the canonicalized
/// path on success to prevent TOCTOU issues.
pub fn validate_job_dir(job_dir: &str) -> Result<PathBuf, String> {
    let path = Path::new(job_dir);
    if !path.exists() {
        return Err(format!("job directory does not exist: {job_dir}"));
    }
    if !path.is_dir() {
        return Err(format!("job directory is not a directory: {job_dir}"));
    }
    // Canonicalize to resolve symlinks and .. traversal
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("failed to canonicalize job directory: {e}"))?;
    if !canonical.is_absolute() {
        return Err("job directory must resolve to an absolute path".to_string());
    }
    // Reject paths containing .. after canonicalization (defensive)
    let canonical_str = canonical.to_string_lossy();
    if canonical_str.contains("..") {
        return Err("job directory contains path traversal".to_string());
    }
    Ok(canonical)
}

// ── Worker command handler ──────────────────────────────────────────────

/// Default model when none is specified via --model flag.
const DEFAULT_WORKER_MODEL: &str = "gpt-4o";
/// Default max iterations when --max-iterations is not specified.
const DEFAULT_MAX_ITERATIONS: u32 = 25;
/// Initial buffer capacity for JSON Lines output (~16 KB).
const INITIAL_OUTPUT_CAPACITY: usize = 16 * 1024;

/// Injected dependencies for `cmd_worker_with_deps`. In production the
/// caller builds a real provider; in tests a mock is injected.
pub struct WorkerDeps {
    pub provider: Arc<dyn LlmProvider>,
}

/// Parse args and validate job directory. Shared by both cmd_worker paths.
fn parse_and_validate(args: &[String], stderr: &mut String) -> Option<(WorkerArgs, PathBuf)> {
    let worker_args = match parse_worker_args(args) {
        Ok(a) => a,
        Err(e) => {
            stderr.push_str(&format!("worker: {e}\n"));
            return None;
        }
    };

    let canonical_dir = match validate_job_dir(&worker_args.job_dir) {
        Ok(p) => p,
        Err(e) => {
            stderr.push_str(&format!("worker: {e}\n"));
            return None;
        }
    };

    Some((worker_args, canonical_dir))
}

/// Handle the `quecto worker` subcommand.
///
/// Parses arguments, validates the job directory, and exits.
/// Without injected deps, emits a plain startup message (stub mode).
pub fn cmd_worker(args: &[String], stdout: &mut String, stderr: &mut String) -> i32 {
    let (worker_args, _canonical_dir) = match parse_and_validate(args, stderr) {
        Some(v) => v,
        None => return 1,
    };

    // Without injected deps, emit a plain startup message (stub mode).
    stdout.push_str(&format!(
        "worker: ready (run={}, job={}, dir={})\n",
        worker_args.run_id, worker_args.job_id, worker_args.job_dir,
    ));
    0
}

/// Handle `quecto worker` with injected dependencies — runs the full
/// agent loop and writes JSON Lines to `stdout`.
pub fn cmd_worker_with_deps(
    args: &[String],
    deps: WorkerDeps,
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let (worker_args, canonical_dir) = match parse_and_validate(args, stderr) {
        Some(v) => v,
        None => return 1,
    };

    let sink = build_default_sink(&worker_args);
    let tool_registry = build_worker_tool_registry(canonical_dir);

    let config = build_loop_config(&worker_args);
    let params = WorkerLoopParams {
        config,
        provider: deps.provider,
        sink: sink.clone() as Arc<dyn WorkerEventSink>,
    };

    // Run the agent loop in a tokio runtime
    let rt = match super::build_tokio_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            stderr.push_str(&format!("worker: failed to build runtime: {e}\n"));
            return 1;
        }
    };

    let result = rt.block_on(run_worker_loop(params, Box::new(tool_registry)));
    collect_output(sink, stdout, stderr);
    result.exit_code
}

/// Build the default `WorkerEventSinkAdapter` for JSON Lines output.
fn build_default_sink(args: &WorkerArgs) -> Arc<WorkerEventSinkAdapter<Vec<u8>>> {
    let emitter = WorkerEventEmitter::new(
        EmitterConfig {
            run_id: args.run_id.clone(),
            job_id: args.job_id.clone(),
            version: "1.0".to_string(),
        },
        Vec::with_capacity(INITIAL_OUTPUT_CAPACITY),
    );
    Arc::new(WorkerEventSinkAdapter::new(emitter))
}

/// Build a `WorkerLoopConfig` from parsed CLI arguments.
fn build_loop_config(args: &WorkerArgs) -> WorkerLoopConfig {
    WorkerLoopConfig {
        run_id: args.run_id.clone(),
        job_id: args.job_id.clone(),
        job_dir: args.job_dir.clone(),
        goal: args.goal.clone(),
        model: args
            .model
            .clone()
            .unwrap_or(DEFAULT_WORKER_MODEL.to_string()),
        max_iterations: args.max_iterations.unwrap_or(DEFAULT_MAX_ITERATIONS),
        ..WorkerLoopConfig::default()
    }
}

/// Extract buffered JSON Lines from the sink into stdout.
///
/// Tries `into_writer` (zero-copy) first, falls back to clone if
/// the Arc has other references.
fn collect_output(
    sink: Arc<WorkerEventSinkAdapter<Vec<u8>>>,
    stdout: &mut String,
    stderr: &mut String,
) {
    let buf = match Arc::try_unwrap(sink) {
        Ok(adapter) => adapter.into_writer().unwrap_or_default(),
        Err(shared) => shared.clone_writer().unwrap_or_default(),
    };
    match String::from_utf8(buf) {
        Ok(json_lines) => stdout.push_str(&json_lines),
        Err(e) => {
            stderr.push_str(&format!("worker: non-UTF8 output: {e}\n"));
        }
    }
}

/// Help text for the worker subcommand.
pub fn worker_help_text() -> &'static str {
    "  worker      Run a coding worker inside nsjail (internal)\n"
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
