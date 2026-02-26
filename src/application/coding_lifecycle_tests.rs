use std::collections::HashMap as StdHashMap;

use super::*;
use crate::application::coding_coordinator::CoordinatorPolicy;
use crate::domain::coding_command::RunRequest;
use crate::domain::coding_job::JobState;
use crate::domain::coding_ports::{
    CloneJobParams, RepoMirrorStore, RepoOpResult, RepoValidator, SkillResolver, WorkerEvent,
    WorkerLaunchConfig, WorkerRuntime, WorkerStatus,
};

// ── Test helpers ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
struct TestRepoValidator;

impl RepoValidator for TestRepoValidator {
    fn repo_exists(&self, _repo: &str) -> bool {
        true
    }
    fn ref_exists(&self, _repo: &str, _base_ref: &str) -> bool {
        true
    }
}

#[derive(Debug, Clone, Default)]
struct TestSkillResolver;

impl SkillResolver for TestSkillResolver {
    fn skill_exists(&self, _name: &str) -> bool {
        true
    }
}

struct SucceedingMirror;

impl RepoMirrorStore for SucceedingMirror {
    fn mirror_exists(&self, _repo: &str) -> bool {
        true
    }
    fn create_mirror(&mut self, _repo: &str, _url: &str) -> RepoOpResult {
        RepoOpResult {
            ok: true,
            duration_ms: 50,
            error: None,
            error_code: None,
        }
    }
    fn fetch_mirror(&self, _repo: &str) -> RepoOpResult {
        RepoOpResult {
            ok: true,
            duration_ms: 30,
            error: None,
            error_code: None,
        }
    }
    fn clone_for_job(&self, _params: &CloneJobParams<'_>) -> RepoOpResult {
        RepoOpResult {
            ok: true,
            duration_ms: 100,
            error: None,
            error_code: None,
        }
    }
    fn mirror_path_for_repo(&self, _repo: &str) -> Option<String> {
        Some("mirror".to_string())
    }
    fn remove_job_repo(&self, _job_id: &str) -> bool {
        true
    }
    fn remove_job_repo_keep_artifacts(&self, _job_id: &str) -> bool {
        true
    }
    fn job_repo_path(&self, job_id: &str) -> String {
        format!("/tmp/test-jobs/{job_id}/repo")
    }
}

struct FailingMirror;

impl RepoMirrorStore for FailingMirror {
    fn mirror_exists(&self, _repo: &str) -> bool {
        false
    }
    fn create_mirror(&mut self, _repo: &str, _url: &str) -> RepoOpResult {
        RepoOpResult {
            ok: false,
            duration_ms: 10,
            error: Some("clone error".to_string()),
            error_code: Some("clone_failure".to_string()),
        }
    }
    fn fetch_mirror(&self, _repo: &str) -> RepoOpResult {
        RepoOpResult {
            ok: false,
            duration_ms: 10,
            error: Some("fetch error".to_string()),
            error_code: None,
        }
    }
    fn clone_for_job(&self, _params: &CloneJobParams<'_>) -> RepoOpResult {
        RepoOpResult {
            ok: false,
            duration_ms: 10,
            error: Some("clone error".to_string()),
            error_code: Some("clone_failure".to_string()),
        }
    }
    fn mirror_path_for_repo(&self, _repo: &str) -> Option<String> {
        None
    }
    fn remove_job_repo(&self, _job_id: &str) -> bool {
        false
    }
    fn remove_job_repo_keep_artifacts(&self, _job_id: &str) -> bool {
        false
    }
    fn job_repo_path(&self, job_id: &str) -> String {
        format!("/tmp/test-jobs/{job_id}/repo")
    }
}

/// A test-only runtime where workers stay alive (Running) until
/// explicitly told otherwise via status. Reports Exited on all
/// PIDs once `mark_all_exited` is called.
struct StayAliveRuntime {
    next_pid: u32,
    alive: StdHashMap<u32, bool>,
    killed: StdHashMap<u32, bool>,
}

impl StayAliveRuntime {
    fn new() -> Self {
        Self {
            next_pid: 30000,
            alive: StdHashMap::new(),
            killed: StdHashMap::new(),
        }
    }
}

impl WorkerRuntime for StayAliveRuntime {
    fn launch(&mut self, _config: &WorkerLaunchConfig) -> Result<u32, String> {
        let pid = self.next_pid;
        self.next_pid += 1;
        self.alive.insert(pid, true);
        Ok(pid)
    }
    fn send_command(&mut self, _pid: u32, _cmd: &str) -> Result<(), String> {
        Ok(())
    }
    fn read_event(&mut self, _pid: u32) -> Option<WorkerEvent> {
        None
    }
    fn read_stderr(&mut self, _pid: u32) -> String {
        String::new()
    }
    fn status(&self, pid: u32) -> WorkerStatus {
        if self.killed.get(&pid).copied().unwrap_or(false) {
            WorkerStatus::Killed {
                reason: "killed by coordinator".to_string(),
            }
        } else if self.alive.get(&pid).copied().unwrap_or(false) {
            WorkerStatus::Running
        } else {
            WorkerStatus::Exited { status: 0 }
        }
    }
    fn kill(&mut self, pid: u32) -> Result<(), String> {
        self.alive.insert(pid, false);
        self.killed.insert(pid, true);
        Ok(())
    }
    fn is_alive(&self, pid: u32) -> bool {
        self.alive.get(&pid).copied().unwrap_or(false)
    }
    fn nsjail_args(&self, _config: &WorkerLaunchConfig) -> Vec<String> {
        vec![]
    }
    fn worker_env(
        &self,
        _config: &WorkerLaunchConfig,
    ) -> Vec<crate::domain::coding_ports::WorkerEnvVar> {
        vec![]
    }
    fn cleanup(&mut self, pid: u32) {
        self.alive.remove(&pid);
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// A runtime that immediately reports workers as exited with a status.
struct ImmediateExitRuntime {
    exit_status: i32,
    next_pid: u32,
}

impl ImmediateExitRuntime {
    fn new(exit_status: i32) -> Self {
        Self {
            exit_status,
            next_pid: 20000,
        }
    }
}

impl WorkerRuntime for ImmediateExitRuntime {
    fn launch(&mut self, _config: &WorkerLaunchConfig) -> Result<u32, String> {
        let pid = self.next_pid;
        self.next_pid += 1;
        Ok(pid)
    }
    fn send_command(&mut self, _pid: u32, _cmd: &str) -> Result<(), String> {
        Ok(())
    }
    fn read_event(&mut self, _pid: u32) -> Option<WorkerEvent> {
        None
    }
    fn read_stderr(&mut self, _pid: u32) -> String {
        String::new()
    }
    fn status(&self, _pid: u32) -> WorkerStatus {
        WorkerStatus::Exited {
            status: self.exit_status,
        }
    }
    fn kill(&mut self, _pid: u32) -> Result<(), String> {
        Ok(())
    }
    fn is_alive(&self, _pid: u32) -> bool {
        false
    }
    fn nsjail_args(&self, _config: &WorkerLaunchConfig) -> Vec<String> {
        vec![]
    }
    fn worker_env(
        &self,
        _config: &WorkerLaunchConfig,
    ) -> Vec<crate::domain::coding_ports::WorkerEnvVar> {
        vec![]
    }
    fn cleanup(&mut self, _pid: u32) {}
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// A runtime that always fails to launch.
struct FailingLaunchRuntime;

impl WorkerRuntime for FailingLaunchRuntime {
    fn launch(&mut self, _config: &WorkerLaunchConfig) -> Result<u32, String> {
        Err("launch error: binary not found".to_string())
    }
    fn send_command(&mut self, _pid: u32, _cmd: &str) -> Result<(), String> {
        Err("not running".to_string())
    }
    fn read_event(&mut self, _pid: u32) -> Option<WorkerEvent> {
        None
    }
    fn read_stderr(&mut self, _pid: u32) -> String {
        String::new()
    }
    fn status(&self, _pid: u32) -> WorkerStatus {
        WorkerStatus::Killed {
            reason: "never launched".to_string(),
        }
    }
    fn kill(&mut self, _pid: u32) -> Result<(), String> {
        Err("unknown PID".to_string())
    }
    fn is_alive(&self, _pid: u32) -> bool {
        false
    }
    fn nsjail_args(&self, _config: &WorkerLaunchConfig) -> Vec<String> {
        vec![]
    }
    fn worker_env(
        &self,
        _config: &WorkerLaunchConfig,
    ) -> Vec<crate::domain::coding_ports::WorkerEnvVar> {
        vec![]
    }
    fn cleanup(&mut self, _pid: u32) {}
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

fn test_request(goal: &str) -> RunRequest {
    RunRequest {
        goal: goal.to_string(),
        repo: "test/repo".to_string(),
        base_ref: "main".to_string(),
        priority: Default::default(),
        profile: "default".to_string(),
        max_wall_seconds: None,
        labels: vec![],
        skills: vec![],
    }
}

fn make_coord() -> CodingCoordinator<TestRepoValidator, TestSkillResolver> {
    CodingCoordinator::new(
        TestRepoValidator,
        TestSkillResolver,
        CoordinatorPolicy::default(),
    )
}

// ── Tests ───────────────────────────────────────────────────────────────
//
// Tick phases are snapshot-based: each tick advances a job at most one
// phase. Tick 1: queued → preparing. Tick 2: preparing → running.
// Tick 3 (with ImmediateExitRuntime): running → succeeded/failed.

#[test]
fn test_tick_queued_to_preparing() {
    let mut driver = CodingLifecycleDriver::new(
        make_coord(),
        Box::new(StayAliveRuntime::new()),
        Box::new(SucceedingMirror),
    );
    let job_id = driver.create_job(test_request("fix bug")).unwrap();
    driver.tick(); // queued → preparing
    let job = driver.coordinator().job(&job_id).unwrap();
    assert_eq!(job.state, JobState::Preparing);
}

#[test]
fn test_tick_preparing_to_running() {
    let mut driver = CodingLifecycleDriver::new(
        make_coord(),
        Box::new(StayAliveRuntime::new()),
        Box::new(SucceedingMirror),
    );
    let job_id = driver.create_job(test_request("add feat")).unwrap();
    driver.tick(); // queued → preparing
    driver.tick(); // preparing → running
    let job = driver.coordinator().job(&job_id).unwrap();
    assert_eq!(job.state, JobState::Running);
    assert!(job.worker_pid.is_some());
}

#[test]
fn test_tick_marks_succeeded_on_exit_0() {
    let mut driver = CodingLifecycleDriver::new(
        make_coord(),
        Box::new(ImmediateExitRuntime::new(0)),
        Box::new(SucceedingMirror),
    );
    let job_id = driver.create_job(test_request("refactor")).unwrap();
    driver.tick(); // queued → preparing
    driver.tick(); // preparing → running (launched, running_workers updated)
    driver.tick(); // running → poll sees Exited(0) → succeeded
    let job = driver.coordinator().job(&job_id).unwrap();
    assert_eq!(job.state, JobState::Succeeded);
}

#[test]
fn test_tick_marks_failed_on_exit_1() {
    let mut driver = CodingLifecycleDriver::new(
        make_coord(),
        Box::new(ImmediateExitRuntime::new(1)),
        Box::new(SucceedingMirror),
    );
    let job_id = driver.create_job(test_request("broken")).unwrap();
    driver.tick(); // queued → preparing
    driver.tick(); // preparing → running
    driver.tick(); // running → poll sees Exited(1) → failed
    let job = driver.coordinator().job(&job_id).unwrap();
    assert_eq!(job.state, JobState::Failed);
}

#[test]
fn test_clone_failure_marks_failed() {
    let mut driver = CodingLifecycleDriver::new(
        make_coord(),
        Box::new(ImmediateExitRuntime::new(0)),
        Box::new(FailingMirror),
    );
    let job_id = driver.create_job(test_request("clone fail")).unwrap();
    driver.tick(); // queued → preparing
    driver.tick(); // preparing → clone fails → failed
    let job = driver.coordinator().job(&job_id).unwrap();
    assert_eq!(job.state, JobState::Failed);
    assert!(job.error_detail.as_deref().unwrap().contains("clone"));
}

#[test]
fn test_launch_failure_marks_failed() {
    let mut driver = CodingLifecycleDriver::new(
        make_coord(),
        Box::new(FailingLaunchRuntime),
        Box::new(SucceedingMirror),
    );
    let job_id = driver.create_job(test_request("launch fail")).unwrap();
    driver.tick(); // queued → preparing
    driver.tick(); // preparing → launch fails → failed
    let job = driver.coordinator().job(&job_id).unwrap();
    assert_eq!(job.state, JobState::Failed);
    assert!(job.error_detail.as_deref().unwrap().contains("launch"));
}

#[test]
fn test_cancel_kills_worker() {
    let mut driver = CodingLifecycleDriver::new(
        make_coord(),
        Box::new(StayAliveRuntime::new()),
        Box::new(SucceedingMirror),
    );
    let job_id = driver.create_job(test_request("cancel me")).unwrap();
    driver.tick(); // queued → preparing
    driver.tick(); // preparing → running
    driver.cancel_job(&job_id).unwrap();
    driver.tick(); // canceled → kill worker
    assert!(driver.was_worker_killed(&job_id));
}

#[test]
fn test_multiple_jobs_processed() {
    let mut driver = CodingLifecycleDriver::new(
        make_coord(),
        Box::new(StayAliveRuntime::new()),
        Box::new(SucceedingMirror),
    );
    let id_a = driver.create_job(test_request("job A")).unwrap();
    let id_b = driver.create_job(test_request("job B")).unwrap();
    driver.tick(); // both queued → preparing
    driver.tick(); // both preparing → running
    let job_a = driver.coordinator().job(&id_a).unwrap();
    let job_b = driver.coordinator().job(&id_b).unwrap();
    assert_eq!(job_a.state, JobState::Running);
    assert_eq!(job_b.state, JobState::Running);
}
