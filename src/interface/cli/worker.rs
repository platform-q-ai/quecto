//! `quecto worker` subcommand — coding worker entrypoint.
//!
//! Parses CLI flags, validates the job directory, builds the worker
//! tool registry and event emitter, and runs the coding agent loop.
//! Designed to run inside nsjail with JSON Lines IPC.

use std::path::Path;

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

/// Validate that the job directory exists and is a directory.
pub fn validate_job_dir(job_dir: &str) -> Result<(), String> {
    let path = Path::new(job_dir);
    if !path.exists() {
        return Err(format!("job directory does not exist: {job_dir}"));
    }
    if !path.is_dir() {
        return Err(format!("job directory is not a directory: {job_dir}"));
    }
    Ok(())
}

// ── Worker command handler ──────────────────────────────────────────────

/// Handle the `quecto worker` subcommand.
///
/// Parses arguments, validates the job directory, and exits.
/// The full agent loop integration is wired in a later feature.
pub fn cmd_worker(args: &[String], stdout: &mut String, stderr: &mut String) -> i32 {
    let worker_args = match parse_worker_args(args) {
        Ok(a) => a,
        Err(e) => {
            stderr.push_str(&format!("worker: {e}\n"));
            return 1;
        }
    };

    if let Err(e) = validate_job_dir(&worker_args.job_dir) {
        stderr.push_str(&format!("worker: {e}\n"));
        return 1;
    }

    // For now, emit a startup message. Full agent loop is wired in
    // the coordinator-worker lifecycle feature.
    stdout.push_str(&format!(
        "worker: ready (run={}, job={}, dir={})\n",
        worker_args.run_id, worker_args.job_id, worker_args.job_dir,
    ));
    0
}

/// Help text for the worker subcommand.
pub fn worker_help_text() -> &'static str {
    "  worker      Run a coding worker inside nsjail (internal)\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(flags: &[(&str, &str)]) -> Vec<String> {
        flags
            .iter()
            .flat_map(|(k, v)| vec![k.to_string(), v.to_string()])
            .collect()
    }

    #[test]
    fn test_parse_all_required() {
        let a = args(&[
            ("--run-id", "r1"),
            ("--job-id", "j1"),
            ("--job-dir", "/tmp/x"),
            ("--goal", "fix"),
        ]);
        let result = parse_worker_args(&a).unwrap();
        assert_eq!(result.run_id, "r1");
        assert_eq!(result.job_id, "j1");
        assert_eq!(result.job_dir, "/tmp/x");
        assert_eq!(result.goal, "fix");
        assert!(result.model.is_none());
        assert!(result.max_iterations.is_none());
    }

    #[test]
    fn test_parse_with_optional() {
        let a = args(&[
            ("--run-id", "r1"),
            ("--job-id", "j1"),
            ("--job-dir", "/tmp/x"),
            ("--goal", "fix"),
            ("--model", "gpt-4o"),
            ("--max-iterations", "50"),
        ]);
        let result = parse_worker_args(&a).unwrap();
        assert_eq!(result.model, Some("gpt-4o".to_string()));
        assert_eq!(result.max_iterations, Some(50));
    }

    #[test]
    fn test_missing_run_id() {
        let a = args(&[
            ("--job-id", "j1"),
            ("--job-dir", "/tmp/x"),
            ("--goal", "fix"),
        ]);
        let err = parse_worker_args(&a).unwrap_err();
        assert!(err.contains("run-id"));
    }

    #[test]
    fn test_missing_job_id() {
        let a = args(&[
            ("--run-id", "r1"),
            ("--job-dir", "/tmp/x"),
            ("--goal", "fix"),
        ]);
        let err = parse_worker_args(&a).unwrap_err();
        assert!(err.contains("job-id"));
    }

    #[test]
    fn test_missing_job_dir() {
        let a = args(&[("--run-id", "r1"), ("--job-id", "j1"), ("--goal", "fix")]);
        let err = parse_worker_args(&a).unwrap_err();
        assert!(err.contains("job-dir"));
    }

    #[test]
    fn test_missing_goal() {
        let a = args(&[
            ("--run-id", "r1"),
            ("--job-id", "j1"),
            ("--job-dir", "/tmp/x"),
        ]);
        let err = parse_worker_args(&a).unwrap_err();
        assert!(err.contains("goal"));
    }

    #[test]
    fn test_unknown_flag() {
        let a = args(&[
            ("--run-id", "r1"),
            ("--job-id", "j1"),
            ("--job-dir", "/tmp/x"),
            ("--goal", "fix"),
            ("--bad-flag", "oops"),
        ]);
        let err = parse_worker_args(&a).unwrap_err();
        assert!(err.contains("bad-flag"));
    }

    #[test]
    fn test_validate_existing_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(validate_job_dir(tmp.path().to_str().unwrap()).is_ok());
    }

    #[test]
    fn test_validate_nonexistent_dir() {
        let err = validate_job_dir("/tmp/nonexistent-quecto-test-99999").unwrap_err();
        assert!(err.contains("does not exist"));
    }

    #[test]
    fn test_cmd_worker_success() {
        let tmp = tempfile::TempDir::new().unwrap();
        let a = args(&[
            ("--run-id", "r1"),
            ("--job-id", "j1"),
            ("--job-dir", tmp.path().to_str().unwrap()),
            ("--goal", "fix"),
        ]);
        let mut stdout = String::new();
        let mut stderr = String::new();
        let code = cmd_worker(&a, &mut stdout, &mut stderr);
        assert_eq!(code, 0);
        assert!(stdout.contains("ready"));
    }

    #[test]
    fn test_cmd_worker_bad_args() {
        let a: Vec<String> = vec![];
        let mut stdout = String::new();
        let mut stderr = String::new();
        let code = cmd_worker(&a, &mut stdout, &mut stderr);
        assert_eq!(code, 1);
        assert!(!stderr.is_empty());
    }
}
