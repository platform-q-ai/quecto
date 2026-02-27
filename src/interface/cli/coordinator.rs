//! `quecto coordinator` subcommand — coordinator entrypoint.
//!
//! Runs the coordinator inbox polling loop: reads commands from inbox,
//! dispatches to `CodingJobService`, writes responses to outbox, writes
//! periodic state snapshots, and exits on shutdown command or SIGTERM.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::application::coordinator_inbox;
use crate::domain::coding_ipc::CoordinatorIpc;
use crate::domain::coding_ports::CodingJobService;
use crate::infrastructure::coding::coordinator_ipc::FileCoordinatorIpc;

// ── Parsed arguments ────────────────────────────────────────────────────

/// Parsed coordinator command-line arguments.
#[derive(Debug, Clone)]
pub struct CoordinatorArgs {
    /// Path to the IPC directory (coordinator/).
    pub ipc_dir: String,
    /// How often to poll the inbox, in milliseconds.
    pub poll_interval_ms: u64,
}

/// Default poll interval in milliseconds.
const DEFAULT_POLL_INTERVAL_MS: u64 = 500;

/// Parse coordinator flags from a slice of CLI arguments.
pub fn parse_coordinator_args(args: &[String]) -> Result<CoordinatorArgs, String> {
    let mut ipc_dir: Option<String> = None;
    let mut poll_interval_ms: Option<u64> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--ipc-dir" => {
                ipc_dir = Some(require_next(args, &mut i, "--ipc-dir")?);
            }
            "--poll-interval-ms" => {
                let val = require_next(args, &mut i, "--poll-interval-ms")?;
                let n: u64 = val
                    .parse()
                    .map_err(|_| format!("--poll-interval-ms must be a number, got '{val}'"))?;
                poll_interval_ms = Some(n);
            }
            "--help" | "-h" => {
                return Err("coordinator: see documentation for usage".to_string());
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

    let ipc_dir = ipc_dir.ok_or("missing required flag --ipc-dir")?;

    Ok(CoordinatorArgs {
        ipc_dir,
        poll_interval_ms: poll_interval_ms.unwrap_or(DEFAULT_POLL_INTERVAL_MS),
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

// ── Coordinator loop ────────────────────────────────────────────────────

/// Install signal handlers that set a shared shutdown flag on
/// SIGTERM or SIGINT.
///
/// Spawns a background thread running a single-threaded tokio runtime
/// that waits for `ctrl_c()` (SIGINT). For SIGTERM we rely on the
/// default behavior (process killed) combined with the PID liveness
/// check in the delegation tool — if the coordinator dies, the main
/// agent will auto-restart it on the next `coding_job` call.
///
/// Returns an `Arc<AtomicBool>` that the loop checks each iteration.
fn install_signal_handlers() -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    let flag_clone = Arc::clone(&flag);

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        if let Ok(rt) = rt {
            rt.block_on(async {
                let _ = tokio::signal::ctrl_c().await;
                flag_clone.store(true, Ordering::SeqCst);
            });
        }
    });

    flag
}

/// Run the coordinator tick loop until shutdown is requested.
///
/// This is the core polling loop: read inbox, dispatch to service, write
/// outbox, write state, sleep, repeat. Exits when `tick()` returns
/// `shutdown_requested: true` or when a SIGTERM/SIGINT signal is received.
pub fn run_coordinator_loop(
    ipc: &dyn CoordinatorIpc,
    service: &mut dyn CodingJobService,
    poll_interval: std::time::Duration,
) -> i32 {
    let shutdown = install_signal_handlers();
    run_coordinator_loop_with_flag(ipc, service, poll_interval, &shutdown)
}

/// Inner loop that checks both the IPC shutdown command and an external
/// `AtomicBool` flag (set by signal handlers or tests).
pub fn run_coordinator_loop_with_flag(
    ipc: &dyn CoordinatorIpc,
    service: &mut dyn CodingJobService,
    poll_interval: std::time::Duration,
    shutdown: &AtomicBool,
) -> i32 {
    // Write PID for liveness checks.
    let pid = std::process::id();
    if let Err(e) = ipc.write_pid(pid) {
        tracing::error!("coordinator: failed to write PID: {e}");
        return 1;
    }

    loop {
        // Check external shutdown flag (signal handler or test).
        if shutdown.load(Ordering::SeqCst) {
            tracing::info!("coordinator: signal received, exiting");
            return 0;
        }

        match coordinator_inbox::tick(ipc, service) {
            Ok(result) => {
                if result.shutdown_requested {
                    tracing::info!("coordinator: shutdown requested, exiting");
                    return 0;
                }
            }
            Err(e) => {
                tracing::error!("coordinator: tick error: {e}");
                // Continue running — transient errors should not kill the loop.
            }
        }
        std::thread::sleep(poll_interval);
    }
}

// ── CLI command handler ─────────────────────────────────────────────────

/// Handle the `quecto coordinator` subcommand with full lifecycle stack.
///
/// Parses args, builds `FileCoordinatorIpc` and `CodingJobService`, then
/// runs the polling loop. This is the production entry point called from
/// `cli/mod.rs`.
pub fn cmd_coordinator(
    ctx: &super::CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let coord_args = match parse_coordinator_args(args) {
        Ok(a) => a,
        Err(e) => {
            stderr.push_str(&format!("coordinator: {e}\n"));
            return 1;
        }
    };

    let ipc_dir = PathBuf::from(&coord_args.ipc_dir);
    let ipc = match FileCoordinatorIpc::new(&ipc_dir) {
        Ok(ipc) => ipc,
        Err(e) => {
            stderr.push_str(&format!("coordinator: failed to init IPC: {e}\n"));
            return 1;
        }
    };

    // Build the full CodingJobService stack.
    let base_dir = ctx.base_dir();
    let workspace = base_dir.join("workspace");

    let service = match build_coordinator_service(&workspace, &base_dir) {
        Ok(svc) => svc,
        Err(e) => {
            stderr.push_str(&format!("coordinator: {e}\n"));
            return 1;
        }
    };
    let mut service = service;

    let poll_interval = std::time::Duration::from_millis(coord_args.poll_interval_ms);

    stdout.push_str(&format!(
        "coordinator: ready (ipc_dir={}, poll={}ms, pid={})\n",
        coord_args.ipc_dir,
        coord_args.poll_interval_ms,
        std::process::id(),
    ));

    run_coordinator_loop(&ipc, service.as_mut(), poll_interval)
}

/// Build the `CodingJobService` for the coordinator process.
///
/// Creates the same stack as `build_coding_lifecycle` but returns a
/// boxed `CodingJobService` instead of registering a tool.
fn build_coordinator_service(
    workspace: &std::path::Path,
    base_dir: &std::path::Path,
) -> Result<Box<dyn CodingJobService>, String> {
    use crate::application::coding_coordinator::{CodingCoordinator, CoordinatorPolicy};
    use crate::infrastructure::coding::nsjail_runtime::{NsjailRuntimeConfig, NsjailWorkerRuntime};
    use crate::infrastructure::coding::repo_mirror::FileRepoMirrorStore;

    let repo_validator = WorkspaceRepoValidator::new(workspace.to_path_buf());
    let skill_resolver = WorkspaceSkillResolver::new(workspace.to_path_buf());
    let coordinator = CodingCoordinator::new(
        repo_validator,
        skill_resolver,
        CoordinatorPolicy {
            skill_denylist: Vec::new(),
            skill_allowlist: Vec::new(),
            max_retained_jobs: Some(512),
        },
    );

    let cache_dir = base_dir.join("coding");
    let mirror = Box::new(FileRepoMirrorStore::with_workspace(
        cache_dir,
        workspace.to_path_buf(),
    ));
    let mut nsjail_config = NsjailRuntimeConfig::default();
    nsjail_config.command_override = Some(vec![
        nsjail_config.quecto_binary.clone(),
        "worker".to_string(),
    ]);
    let runtime = Box::new(NsjailWorkerRuntime::new(nsjail_config));
    let driver = CodingLifecycleDriver::new(coordinator, runtime, mirror);
    let shared = Arc::new(Mutex::new(driver));

    let repo_creator = Box::new(WorkspaceRepoCreator::new(workspace.to_path_buf()));

    Ok(Box::new(CoordinatorJobService {
        driver: shared,
        repo_creator,
    }))
}

/// `CodingJobService` adapter for the coordinator process.
///
/// Same logic as `DriverJobService` in `shared.rs` but owned by the
/// coordinator process rather than shared via `Arc<Mutex<dyn CodingJobService>>`.
use crate::application::coding_lifecycle::CodingLifecycleDriver;
use crate::domain::coding_command::{
    CancelResponse, CleanupResponse, CommandError, CreateRequest, CreateResponse, ImportRequest,
    ImportResponse, ListRequest, ListResponse, RunRequest, RunResponse, StatusResponse,
};
use crate::domain::coding_ports::RepoCreator;
use crate::infrastructure::coding::runtime_adapters::{
    WorkspaceRepoCreator, WorkspaceRepoValidator, WorkspaceSkillResolver,
};
use std::sync::Mutex;

type SharedDriver =
    Arc<Mutex<CodingLifecycleDriver<WorkspaceRepoValidator, WorkspaceSkillResolver>>>;

struct CoordinatorJobService {
    driver: SharedDriver,
    repo_creator: Box<dyn RepoCreator>,
}

fn lock_driver(
    driver: &SharedDriver,
) -> Result<
    std::sync::MutexGuard<
        '_,
        CodingLifecycleDriver<WorkspaceRepoValidator, WorkspaceSkillResolver>,
    >,
    CommandError,
> {
    driver
        .lock()
        .map_err(|e| CommandError::Internal(format!("driver lock poisoned: {e}")))
}

impl CodingJobService for CoordinatorJobService {
    fn create_repo(&mut self, req: CreateRequest) -> Result<CreateResponse, CommandError> {
        self.repo_creator.validate_name(&req.name)?;
        if self.repo_creator.exists(&req.name) {
            return Err(CommandError::AlreadyExists);
        }
        let path = self
            .repo_creator
            .create(&req.name, req.description.as_deref())?;
        Ok(CreateResponse {
            name: req.name,
            path,
            created: true,
        })
    }

    fn import_repo(&mut self, req: ImportRequest) -> Result<ImportResponse, CommandError> {
        let name = match req.name {
            Some(ref n) => n.clone(),
            None => self.repo_creator.name_from_url(&req.url)?,
        };
        self.repo_creator.validate_name(&name)?;
        if self.repo_creator.exists(&name) {
            return Err(CommandError::AlreadyExists);
        }
        let path = self.repo_creator.import(&req.url, &name)?;
        Ok(ImportResponse {
            name,
            path,
            imported: true,
        })
    }

    fn run(&mut self, req: RunRequest) -> Result<RunResponse, CommandError> {
        let mut guard = lock_driver(&self.driver)?;
        let resp = guard.coordinator_mut().run(req)?;
        guard.tick();
        Ok(resp)
    }

    fn status_by_job_id(&self, job_id: &str) -> Result<StatusResponse, CommandError> {
        let mut guard = lock_driver(&self.driver)?;
        guard.tick();
        guard.coordinator().status_by_job_id(job_id)
    }

    fn status_by_run_id(&self, run_id: &str) -> Result<StatusResponse, CommandError> {
        let mut guard = lock_driver(&self.driver)?;
        guard.tick();
        guard.coordinator().status_by_run_id(run_id)
    }

    fn cancel(&mut self, job_id: &str) -> Result<CancelResponse, CommandError> {
        let mut guard = lock_driver(&self.driver)?;
        let resp = guard.coordinator_mut().cancel(job_id)?;
        guard.tick();
        Ok(resp)
    }

    fn cleanup(
        &mut self,
        job_id: &str,
        keep_artifacts: bool,
    ) -> Result<CleanupResponse, CommandError> {
        let mut guard = lock_driver(&self.driver)?;
        let resp = guard.coordinator_mut().cleanup(job_id, keep_artifacts)?;
        guard.forget_job(job_id);
        Ok(resp)
    }

    fn list(&self, req: &ListRequest) -> ListResponse {
        match lock_driver(&self.driver) {
            Ok(guard) => guard.coordinator().list(req),
            Err(_) => ListResponse { jobs: vec![] },
        }
    }
}

/// Help text for the coordinator subcommand.
pub fn coordinator_help_text() -> &'static str {
    "  coordinator Run the coordinator inbox loop (internal)\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_args_basic() {
        let args = vec!["--ipc-dir".into(), "/tmp/coord".into()];
        let parsed = parse_coordinator_args(&args).unwrap();
        assert_eq!(parsed.ipc_dir, "/tmp/coord");
        assert_eq!(parsed.poll_interval_ms, DEFAULT_POLL_INTERVAL_MS);
    }

    #[test]
    fn test_parse_args_with_poll_interval() {
        let args = vec![
            "--ipc-dir".into(),
            "/tmp/coord".into(),
            "--poll-interval-ms".into(),
            "100".into(),
        ];
        let parsed = parse_coordinator_args(&args).unwrap();
        assert_eq!(parsed.ipc_dir, "/tmp/coord");
        assert_eq!(parsed.poll_interval_ms, 100);
    }

    #[test]
    fn test_parse_args_missing_ipc_dir() {
        let args = vec!["--poll-interval-ms".into(), "200".into()];
        let err = parse_coordinator_args(&args).unwrap_err();
        assert!(err.contains("missing required flag --ipc-dir"));
    }

    #[test]
    fn test_parse_args_unknown_flag() {
        let args = vec![
            "--ipc-dir".into(),
            "/tmp/coord".into(),
            "--unknown-flag".into(),
        ];
        let err = parse_coordinator_args(&args).unwrap_err();
        assert!(err.contains("unknown flag"));
    }

    #[test]
    fn test_parse_args_invalid_poll_interval() {
        let args = vec![
            "--ipc-dir".into(),
            "/tmp/coord".into(),
            "--poll-interval-ms".into(),
            "abc".into(),
        ];
        let err = parse_coordinator_args(&args).unwrap_err();
        assert!(err.contains("must be a number"));
    }

    #[test]
    fn test_parse_args_ipc_dir_missing_value() {
        let args = vec!["--ipc-dir".into()];
        let err = parse_coordinator_args(&args).unwrap_err();
        assert!(err.contains("requires a value"));
    }
}
