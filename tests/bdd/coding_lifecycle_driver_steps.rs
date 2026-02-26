use cucumber::{given, then, when};
use quecto::application::coding_coordinator::CoordinatorPolicy;
use quecto::application::coding_lifecycle::CodingLifecycleDriver;
use quecto::domain::coding_command::RunRequest;
use quecto::domain::coding_event::{EventEnvelope, EventSource};
use quecto::domain::coding_ports::{
    CloneJobParams, RepoMirrorStore, RepoOpResult, WorkerEvent, WorkerRuntime,
};
use quecto::infrastructure::coding::worker_runtime::MockWorkerRuntime;

use crate::{BddRepoValidator, BddSkillResolver, QuectoWorld};

// ── Mock mirror stores ──────────────────────────────────────────────────

struct BddSucceedingMirror;

impl RepoMirrorStore for BddSucceedingMirror {
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

struct BddFailingMirror;

impl RepoMirrorStore for BddFailingMirror {
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

/// A failing runtime where launch always returns an error.
struct BddFailingRuntime;

impl WorkerRuntime for BddFailingRuntime {
    fn launch(
        &mut self,
        _config: &quecto::domain::coding_ports::WorkerLaunchConfig,
    ) -> Result<u32, String> {
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
    fn status(&self, _pid: u32) -> quecto::domain::coding_ports::WorkerStatus {
        quecto::domain::coding_ports::WorkerStatus::Killed {
            reason: "never launched".to_string(),
        }
    }
    fn kill(&mut self, _pid: u32) -> Result<(), String> {
        Err("unknown PID".to_string())
    }
    fn is_alive(&self, _pid: u32) -> bool {
        false
    }
    fn nsjail_args(
        &self,
        _config: &quecto::domain::coding_ports::WorkerLaunchConfig,
    ) -> Vec<String> {
        vec![]
    }
    fn worker_env(
        &self,
        _config: &quecto::domain::coding_ports::WorkerLaunchConfig,
    ) -> Vec<quecto::domain::coding_ports::WorkerEnvVar> {
        vec![]
    }
    fn cleanup(&mut self, _pid: u32) {}
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

type BddDriver = CodingLifecycleDriver<BddRepoValidator, BddSkillResolver>;

fn ld_driver(world: &mut QuectoWorld) -> &mut BddDriver {
    world.ld_driver.as_mut().expect("lifecycle driver")
}

fn ld_mock_runtime(world: &mut QuectoWorld) -> &mut MockWorkerRuntime {
    ld_driver(world)
        .runtime_mut()
        .as_any_mut()
        .downcast_mut::<MockWorkerRuntime>()
        .expect("runtime should be MockWorkerRuntime")
}

fn make_bdd_driver(runtime: Box<dyn WorkerRuntime>, mirror: Box<dyn RepoMirrorStore>) -> BddDriver {
    let validator = BddRepoValidator {
        valid_repos: vec!["test/repo".to_string()],
        valid_refs: vec![("test/repo".to_string(), "main".to_string())],
    };
    let resolver = BddSkillResolver::default();
    let policy = CoordinatorPolicy::default();
    let coord = quecto::application::coding_coordinator::CodingCoordinator::new(
        validator, resolver, policy,
    );
    CodingLifecycleDriver::new(coord, runtime, mirror)
}

fn create_queued_job(world: &mut QuectoWorld, goal: &str) {
    let req = RunRequest {
        goal: goal.to_string(),
        repo: "test/repo".to_string(),
        base_ref: "main".to_string(),
        priority: Default::default(),
        profile: "default".to_string(),
        max_wall_seconds: None,
        labels: vec![],
        skills: vec![],
    };
    let job_id = ld_driver(world).create_job(req).expect("create job");
    world.ld_job_ids.push(job_id);
}

// ── Given steps ─────────────────────────────────────────────────────────

#[given("a lifecycle driver with a mock runtime and mirror")]
fn given_driver_with_mocks(world: &mut QuectoWorld) {
    world.ld_driver = Some(make_bdd_driver(
        Box::new(MockWorkerRuntime::new()),
        Box::new(BddSucceedingMirror),
    ));
    world.ld_job_ids.clear();
}

#[given("a lifecycle driver with a failing mirror")]
fn given_driver_with_failing_mirror(world: &mut QuectoWorld) {
    world.ld_driver = Some(make_bdd_driver(
        Box::new(MockWorkerRuntime::new()),
        Box::new(BddFailingMirror),
    ));
    world.ld_job_ids.clear();
}

#[given("a lifecycle driver with a failing runtime")]
fn given_driver_with_failing_runtime(world: &mut QuectoWorld) {
    world.ld_driver = Some(make_bdd_driver(
        Box::new(BddFailingRuntime),
        Box::new(BddSucceedingMirror),
    ));
    world.ld_job_ids.clear();
}

#[given(regex = r#"^a queued coding job with goal "([^"]+)"$"#)]
fn given_queued_job(world: &mut QuectoWorld, goal: String) {
    create_queued_job(world, &goal);
}

// ── When steps ──────────────────────────────────────────────────────────

#[when("the driver ticks once")]
fn when_tick_once(world: &mut QuectoWorld) {
    ld_driver(world).tick();
}

#[when("the driver ticks twice")]
fn when_tick_twice(world: &mut QuectoWorld) {
    ld_driver(world).tick();
    ld_driver(world).tick();
}

#[when("the mock worker exits with status 0")]
fn when_worker_exits_0(world: &mut QuectoWorld) {
    let job_id = world.ld_job_ids.last().expect("job id").clone();
    let pid = ld_driver(world)
        .coordinator()
        .job(&job_id)
        .expect("job")
        .worker_pid
        .expect("worker pid");
    ld_mock_runtime(world).simulate_exit(pid, 0);
}

#[when("the mock worker exits with status 1")]
fn when_worker_exits_1(world: &mut QuectoWorld) {
    let job_id = world.ld_job_ids.last().expect("job id").clone();
    let pid = ld_driver(world)
        .coordinator()
        .job(&job_id)
        .expect("job")
        .worker_pid
        .expect("worker pid");
    ld_mock_runtime(world).simulate_exit(pid, 1);
}

#[when(regex = r#"^the mock worker emits a "([^"]+)" event$"#)]
fn when_worker_emits_event(world: &mut QuectoWorld, event_type: String) {
    let job_id = world.ld_job_ids.last().expect("job id").clone();
    let job = ld_driver(world).coordinator().job(&job_id).expect("job");
    let pid = job.worker_pid.expect("worker pid");
    let run_id = job.run_id.clone();
    let jid = job.job_id.clone();

    let envelope = EventEnvelope {
        v: "1.0".to_string(),
        ts: "2026-01-01T00:00:00Z".to_string(),
        run_id,
        job_id: jid,
        source: EventSource::Worker,
        event_type: event_type.clone(),
        seq: 1,
        payload: serde_json::json!({"level": "info", "message": "test"}),
    };

    ld_mock_runtime(world).inject_event(pid, WorkerEvent::Valid(envelope));
}

#[when("the job is canceled")]
fn when_job_canceled(world: &mut QuectoWorld) {
    let job_id = world.ld_job_ids.last().expect("job id").clone();
    ld_driver(world).cancel_job(&job_id).expect("cancel");
}

// ── Then steps ──────────────────────────────────────────────────────────

#[then(regex = r#"^the job should be in "([^"]+)" state$"#)]
fn then_job_in_state(world: &mut QuectoWorld, expected: String) {
    let job_id = world.ld_job_ids.last().expect("job id").clone();
    let job = ld_driver(world)
        .coordinator()
        .job(&job_id)
        .expect("job should exist");
    assert_eq!(
        job.state.to_string(),
        expected,
        "expected job in '{expected}' state, got '{}'",
        job.state
    );
}

#[then("the job should have a worker PID assigned")]
fn then_job_has_pid(world: &mut QuectoWorld) {
    let job_id = world.ld_job_ids.last().expect("job id").clone();
    let job = ld_driver(world)
        .coordinator()
        .job(&job_id)
        .expect("job should exist");
    assert!(
        job.worker_pid.is_some(),
        "expected job to have a worker PID assigned"
    );
}

#[then("the coordinator should have received the worker event")]
fn then_received_event(world: &mut QuectoWorld) {
    let events = ld_driver(world).coordinator().events();
    let has_worker_event = events.iter().any(|e| e.source == EventSource::Worker);
    assert!(
        has_worker_event,
        "expected coordinator to have received a worker event"
    );
}

#[then(regex = r#"^the job error should contain "([^"]+)"$"#)]
fn then_error_contains(world: &mut QuectoWorld, expected: String) {
    let job_id = world.ld_job_ids.last().expect("job id").clone();
    let job = ld_driver(world)
        .coordinator()
        .job(&job_id)
        .expect("job should exist");
    let detail = job.error_detail.as_deref().unwrap_or("");
    assert!(
        detail.to_lowercase().contains(&expected.to_lowercase()),
        "expected error_detail to contain '{expected}', got '{detail}'"
    );
}

#[then("the mock worker should have been killed")]
fn then_worker_killed(world: &mut QuectoWorld) {
    let job_id = world.ld_job_ids.last().expect("job id").clone();
    assert!(
        ld_driver(world).was_worker_killed(&job_id),
        "expected worker to have been killed"
    );
}

#[then(regex = r#"^both jobs should be in "([^"]+)" state$"#)]
fn then_both_jobs_in_state(world: &mut QuectoWorld, expected: String) {
    let ids = world.ld_job_ids.clone();
    assert!(ids.len() >= 2, "expected at least 2 jobs");
    for job_id in &ids {
        let job = ld_driver(world)
            .coordinator()
            .job(job_id)
            .expect("job should exist");
        assert_eq!(
            job.state.to_string(),
            expected,
            "expected job {} in '{expected}' state, got '{}'",
            job_id,
            job.state
        );
    }
}
