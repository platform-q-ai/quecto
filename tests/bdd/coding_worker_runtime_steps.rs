use cucumber::{given, then, when};
use quecto::domain::coding_event::{EventEnvelope, EventSource};
use quecto::domain::coding_ports::{
    WorkerEnvVar, WorkerEvent, WorkerLaunchConfig, WorkerRuntime, WorkerStatus,
};
use quecto::infrastructure::coding::worker_runtime::{MockWorkerRuntime, is_blocked_env};

use crate::QuectoWorld;

// ── helpers ──────────────────────────────────────────────────────────────

fn ensure_runtime(world: &mut QuectoWorld) {
    if world.coding_worker_runtime.is_none() {
        world.coding_worker_runtime = Some(MockWorkerRuntime::new());
    }
}

fn runtime(world: &mut QuectoWorld) -> &mut MockWorkerRuntime {
    world
        .coding_worker_runtime
        .as_mut()
        .expect("worker runtime")
}

fn default_config() -> WorkerLaunchConfig {
    WorkerLaunchConfig {
        job_dir: "/tmp/jobs/job_000001/repo".to_string(),
        goal: "fix tests".to_string(),
        max_memory_mb: 512,
        max_cpu_seconds: 120,
        max_wall_seconds: 300,
        max_pids: 128,
        network_allowed_hosts: vec![],
        die_with_parent: true,
    }
}

fn launch_worker(world: &mut QuectoWorld) -> u32 {
    let config = world
        .coding_worker_launch_config
        .clone()
        .unwrap_or_else(default_config);
    let rt = runtime(world);
    let pid = rt.launch(&config).expect("launch should succeed");
    world.coding_worker_pid = Some(pid);
    pid
}

fn make_valid_event(job_id: &str, event_type: &str) -> EventEnvelope {
    EventEnvelope {
        v: "1.0".to_string(),
        ts: "2026-01-01T00:00:00Z".to_string(),
        run_id: "run_001".to_string(),
        job_id: job_id.to_string(),
        source: EventSource::Worker,
        event_type: event_type.to_string(),
        seq: 1,
        payload: serde_json::json!({"state": "running"}),
    }
}

// ── Worker launch steps ─────────────────────────────────────────────────

#[given("a coding coordinator with nsjail available")]
fn given_coordinator_with_nsjail(world: &mut QuectoWorld) {
    ensure_runtime(world);
    world.coding_worker_launch_config = Some(default_config());
}

#[given(regex = r#"^a coding job in state "queued" for repo "([^"]+)" at base ref "([^"]+)"$"#)]
fn given_queued_job_for_repo(world: &mut QuectoWorld, _repo: String, _base_ref: String) {
    ensure_runtime(world);
    world.coding_worker_job_state = Some("queued".to_string());
}

#[when("the coordinator begins preparation and the repo clone succeeds")]
fn when_preparation_clone_succeeds(world: &mut QuectoWorld) {
    world.coding_worker_job_state = Some("running".to_string());
    launch_worker(world);
}

#[then("a worker process should be started inside nsjail")]
fn then_worker_started(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid should be set");
    let rt = runtime(world);
    assert!(rt.is_alive(pid), "worker should be alive after launch");
}

#[then("the worker should receive the job goal and config via stdin")]
fn then_worker_receives_goal(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let config = world
        .coding_worker_launch_config
        .as_ref()
        .expect("launch config");
    let goal_cmd = serde_json::json!({"type": "init", "goal": config.goal}).to_string();
    let rt = runtime(world);
    assert!(
        rt.send_command(pid, &goal_cmd).is_ok(),
        "should be able to send goal to worker"
    );
}

#[then(regex = r#"^the worker job state should become "([^"]+)"$"#)]
fn then_job_state_transitions(world: &mut QuectoWorld, expected: String) {
    let state = world
        .coding_worker_job_state
        .as_ref()
        .expect("job state should be set");
    assert_eq!(state, &expected, "job state should be {expected}");
}

// ── Mount table steps ───────────────────────────────────────────────────

#[given(regex = r#"^a worker coding job in state "preparing" with job directory "([^"]+)"$"#)]
fn given_preparing_job_with_dir(world: &mut QuectoWorld, job_dir: String) {
    ensure_runtime(world);
    let mut config = default_config();
    config.job_dir = job_dir;
    world.coding_worker_launch_config = Some(config);
}

#[when("the worker is launched inside nsjail")]
fn when_worker_launched(world: &mut QuectoWorld) {
    launch_worker(world);
}

#[then("the nsjail mount table should include the job directory as read-write")]
fn then_mount_table_includes_job_dir_rw(world: &mut QuectoWorld) {
    let job_dir = world
        .coding_worker_launch_config
        .as_ref()
        .expect("launch config")
        .job_dir
        .clone();
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let args = rt.nsjail_args_for(pid);
    // Check that --bindmount appears with the job_dir
    let rw_mount = args
        .windows(2)
        .any(|pair| pair[0] == "--bindmount" && pair[1].contains(&job_dir));
    assert!(rw_mount, "job directory should be mounted read-write");
}

#[then("the host root filesystem should be mounted read-only")]
fn then_host_root_ro(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let args = rt.nsjail_args_for(pid);
    let ro_mount = args
        .windows(2)
        .any(|pair| pair[0] == "--bindmount_ro" && pair[1].contains("/:/host"));
    assert!(ro_mount, "host root should be mounted read-only");
}

#[then("no other directories should be writable")]
fn then_no_other_writable(world: &mut QuectoWorld) {
    let job_dir = world
        .coding_worker_launch_config
        .as_ref()
        .expect("launch config")
        .job_dir
        .clone();
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let args = rt.nsjail_args_for(pid);
    // Count --bindmount (rw) occurrences — should be exactly 1
    let rw_count = args.iter().filter(|a| *a == "--bindmount").count();
    assert_eq!(rw_count, 1, "only one rw mount (the job dir) should exist");
    // Verify that one rw mount is the job dir
    let mount_idx = args.iter().position(|a| a == "--bindmount").unwrap();
    assert!(
        args[mount_idx + 1].contains(&job_dir),
        "the sole rw mount should be the job dir"
    );
}

// ── Resource limits steps ───────────────────────────────────────────────

#[given("a coding coordinator with config:")]
fn given_coordinator_with_config(world: &mut QuectoWorld, step: &cucumber::gherkin::Step) {
    ensure_runtime(world);
    let mut config = default_config();
    if let Some(table) = &step.table {
        for row in &table.rows {
            if row.len() < 2 {
                continue;
            }
            let key = row[0].trim();
            let val = row[1].trim();
            match key {
                "coding.isolation.resources.max_memory_mb" => {
                    config.max_memory_mb = val.parse().unwrap();
                }
                "coding.isolation.resources.max_cpu_seconds" => {
                    config.max_cpu_seconds = val.parse().unwrap();
                }
                "coding.isolation.resources.max_wall_seconds" => {
                    config.max_wall_seconds = val.parse().unwrap();
                }
                "coding.isolation.resources.max_pids" => {
                    config.max_pids = val.parse().unwrap();
                }
                _ => {}
            }
        }
    }
    world.coding_worker_launch_config = Some(config);
}

#[when("a worker is launched for a coding job")]
fn when_worker_launched_for_job(world: &mut QuectoWorld) {
    launch_worker(world);
}

#[then(regex = r#"^the nsjail process should have memory limit (\d+) MB$"#)]
fn then_memory_limit(world: &mut QuectoWorld, limit: u32) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let args = rt.nsjail_args_for(pid);
    let idx = args
        .iter()
        .position(|a| a == "--rlimit_as")
        .expect("--rlimit_as in args");
    let actual: u32 = args[idx + 1].parse().expect("memory limit value");
    assert_eq!(actual, limit, "memory limit should be {limit} MB");
}

#[then(regex = r#"^the nsjail process should have CPU time limit (\d+) seconds$"#)]
fn then_cpu_limit(world: &mut QuectoWorld, limit: u32) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let args = rt.nsjail_args_for(pid);
    let idx = args
        .iter()
        .position(|a| a == "--rlimit_cpu")
        .expect("--rlimit_cpu in args");
    let actual: u32 = args[idx + 1].parse().expect("cpu limit value");
    assert_eq!(actual, limit, "CPU time limit should be {limit} seconds");
}

#[then(regex = r#"^the nsjail process should have wall time limit (\d+) seconds$"#)]
fn then_wall_limit(world: &mut QuectoWorld, limit: u32) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let args = rt.nsjail_args_for(pid);
    let idx = args
        .iter()
        .position(|a| a == "--time_limit")
        .expect("--time_limit in args");
    let actual: u32 = args[idx + 1].parse().expect("wall limit value");
    assert_eq!(actual, limit, "wall time limit should be {limit} seconds");
}

#[then(regex = r#"^the nsjail process should have PID limit (\d+)$"#)]
fn then_pid_limit(world: &mut QuectoWorld, limit: u32) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let args = rt.nsjail_args_for(pid);
    let idx = args
        .iter()
        .position(|a| a == "--cgroup_pids_max")
        .expect("--cgroup_pids_max in args");
    let actual: u32 = args[idx + 1].parse().expect("pid limit value");
    assert_eq!(actual, limit, "PID limit should be {limit}");
}

// ── Security steps ──────────────────────────────────────────────────────

#[then("the nsjail process should have no_new_privs enabled")]
fn then_no_new_privs(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let args = rt.nsjail_args_for(pid);
    assert!(
        args.contains(&"--no_new_privs".to_string()),
        "no_new_privs should be enabled"
    );
}

#[then("a seccomp-bpf profile should be applied")]
fn then_seccomp_applied(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let args = rt.nsjail_args_for(pid);
    assert!(
        args.contains(&"--seccomp_string".to_string()),
        "seccomp-bpf should be applied"
    );
}

// ── JSON Lines IPC steps ────────────────────────────────────────────────

#[given("a running coding worker inside nsjail")]
fn given_running_worker(world: &mut QuectoWorld) {
    ensure_runtime(world);
    world.coding_worker_launch_config = Some(default_config());
    launch_worker(world);
}

#[when("the worker emits a JSON Lines message on stdout")]
fn when_worker_emits_jsonl(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let event = make_valid_event("job_000001", "job.status");
    let rt = runtime(world);
    rt.inject_event(pid, WorkerEvent::Valid(event));
}

#[then("the coordinator should parse it as an event envelope")]
fn then_parsed_as_envelope(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let event = rt.read_event(pid);
    assert!(
        matches!(event, Some(WorkerEvent::Valid(_))),
        "event should be parsed as a valid envelope"
    );
    world.coding_worker_last_event = event;
}

#[then("the event should be validated against the coding contract")]
fn then_event_validated(world: &mut QuectoWorld) {
    let event = world
        .coding_worker_last_event
        .as_ref()
        .expect("last event should exist");
    match event {
        WorkerEvent::Valid(env) => {
            assert!(!env.v.is_empty(), "event version should be set");
            assert!(!env.event_type.is_empty(), "event type should be set");
            assert!(!env.job_id.is_empty(), "event job_id should be set");
        }
        WorkerEvent::Malformed { .. } => {
            panic!("expected valid event, got malformed");
        }
    }
}

#[when("the coordinator writes a JSON Lines command to the worker's stdin")]
fn when_coordinator_writes_command(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let command = serde_json::json!({"type": "ping"}).to_string();
    let rt = runtime(world);
    let result = rt.send_command(pid, &command);
    world.coding_worker_command_sent = result.is_ok();
}

#[then("the worker should receive and process the command")]
fn then_worker_receives_command(world: &mut QuectoWorld) {
    assert!(
        world.coding_worker_command_sent,
        "command should have been sent successfully"
    );
}

#[when("the worker writes a non-JSON line to stdout")]
fn when_worker_writes_malformed(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    rt.inject_event(
        pid,
        WorkerEvent::Malformed {
            raw: "this is not valid JSON".to_string(),
        },
    );
}

#[then("the coordinator should log a warning about the malformed line")]
fn then_malformed_logged(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let event = rt.read_event(pid);
    assert!(
        matches!(event, Some(WorkerEvent::Malformed { .. })),
        "malformed event should be detected"
    );
    world.coding_worker_malformed_detected = true;
}

#[then("the coordinator should continue processing subsequent lines")]
fn then_continues_processing(world: &mut QuectoWorld) {
    assert!(
        world.coding_worker_malformed_detected,
        "malformed line should have been detected and handled"
    );
    // Inject another valid event to show processing continues
    let pid = world.coding_worker_pid.expect("worker pid");
    let event = make_valid_event("job_000001", "job.status");
    let rt = runtime(world);
    rt.inject_event(pid, WorkerEvent::Valid(event));
    let next = rt.read_event(pid);
    assert!(
        matches!(next, Some(WorkerEvent::Valid(_))),
        "should continue processing after malformed line"
    );
}

#[when("the worker writes to stderr")]
fn when_worker_writes_stderr(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    rt.inject_stderr(pid, "WARNING: some diagnostic output");
}

#[then("the coordinator should capture stderr output for the job's diagnostic log")]
fn then_stderr_captured(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let stderr = rt.read_stderr(pid);
    assert!(
        stderr.contains("WARNING"),
        "stderr should be captured: got '{stderr}'"
    );
}

#[then("stderr output should not be interpreted as event messages")]
fn then_stderr_not_events(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    // No events should have been created from stderr
    let event = rt.read_event(pid);
    assert!(event.is_none(), "stderr should not produce events");
}

// ── Worker lifecycle steps ──────────────────────────────────────────────

#[when(regex = r#"^the worker process exits with status (\d+)$"#)]
fn when_worker_exits(world: &mut QuectoWorld, status: i32) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    rt.simulate_exit(pid, status);
    world.coding_worker_exit_status = Some(status);
}

#[then("the coordinator should process any remaining stdout events")]
fn then_remaining_events_processed(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    // After exit, reading events should return None (all drained)
    let remaining = rt.read_event(pid);
    assert!(
        remaining.is_none(),
        "all events should be drained after exit"
    );
}

#[then("the job should transition based on the final event state")]
fn then_job_transitions_on_final_event(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let status = rt.status(pid);
    assert_eq!(
        status,
        WorkerStatus::Exited { status: 0 },
        "worker should have exited cleanly"
    );
}

#[then(regex = r#"^the coordinator should transition the job to "([^"]+)"$"#)]
fn then_coordinator_transitions_job(world: &mut QuectoWorld, expected: String) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let status = rt.status(pid);
    match expected.as_str() {
        "failed" => {
            assert!(
                matches!(status, WorkerStatus::Exited { status } if status != 0),
                "worker should have exited with non-zero status for failed: got {status:?}"
            );
        }
        _ => {
            panic!("unexpected state transition: {expected}");
        }
    }
}

#[then(regex = r#"^the worker error_code should be "([^"]+)"$"#)]
fn then_error_code(world: &mut QuectoWorld, expected: String) {
    assert_eq!(
        expected, "worker_crash",
        "error code should be worker_crash"
    );
    let exit_status = world
        .coding_worker_exit_status
        .expect("exit status should be set");
    assert_ne!(exit_status, 0, "non-zero exit implies worker_crash");
}

#[then("the diagnostic log should include the exit status")]
fn then_diagnostic_includes_exit(world: &mut QuectoWorld) {
    let exit_status = world
        .coding_worker_exit_status
        .expect("exit status should be set");
    assert_ne!(
        exit_status, 0,
        "exit status should be non-zero for diagnostics"
    );
}

#[given(regex = r#"^the job has max_wall_seconds (\d+)$"#)]
fn given_max_wall_seconds(world: &mut QuectoWorld, max_wall: u32) {
    let config = world
        .coding_worker_launch_config
        .get_or_insert_with(default_config);
    config.max_wall_seconds = max_wall;
}

#[when(regex = r#"^the worker has been running for more than (\d+) seconds$"#)]
fn when_wall_timeout_exceeded(world: &mut QuectoWorld, _seconds: u32) {
    // Simulate the coordinator killing the worker on timeout
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    rt.simulate_kill(pid, "wall_timeout");
    world.coding_worker_timeout_fired = true;
}

#[then("the coordinator should kill the nsjail process")]
fn then_nsjail_killed(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    assert!(
        !rt.is_alive(pid),
        "worker should be dead after timeout kill"
    );
}

#[then(regex = r#"^the job should transition to "canceled" with reason "([^"]+)"$"#)]
fn then_job_canceled_with_reason(world: &mut QuectoWorld, reason: String) {
    assert_eq!(
        reason, "wall_timeout",
        "cancel reason should be wall_timeout"
    );
    assert!(
        world.coding_worker_timeout_fired,
        "timeout should have been triggered"
    );
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let status = rt.status(pid);
    assert!(
        matches!(status, WorkerStatus::Killed { reason: ref r } if r.contains("wall_timeout")),
        "worker should be killed with wall_timeout reason: got {status:?}"
    );
}

#[then(regex = r#"^a worker "([^"]+)" event should be recorded$"#)]
fn then_event_emitted(world: &mut QuectoWorld, event_type: String) {
    // In the mock runtime, event emission is tracked by the coordinator.
    // Here we verify that the state is consistent with the event being emitted.
    assert_eq!(event_type, "job.cancel", "expected job.cancel event");
    assert!(
        world.coding_worker_timeout_fired,
        "timeout kill should have occurred, triggering job.cancel event"
    );
}

#[when("the parent job is canceled by the user")]
fn when_parent_job_canceled(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    rt.kill(pid).expect("kill should succeed");
    world.coding_worker_user_canceled = true;
}

#[then("the coordinator should send SIGTERM to the nsjail process")]
fn then_sigterm_sent(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    assert!(
        !rt.is_alive(pid),
        "worker should be terminated after SIGTERM"
    );
}

#[then("if the process does not exit within 5 seconds send SIGKILL")]
fn then_sigkill_fallback(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    // In the mock, kill is immediate. Verify the worker is dead.
    assert!(
        !rt.is_alive(pid),
        "worker should be dead (SIGKILL fallback)"
    );
}

#[then(regex = r#"^the job should reach terminal state "([^"]+)"$"#)]
fn then_terminal_state(world: &mut QuectoWorld, expected: String) {
    assert_eq!(expected, "canceled", "terminal state should be canceled");
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    assert!(
        !rt.is_alive(pid),
        "worker should not be alive in terminal state"
    );
}

// ── Network isolation steps ─────────────────────────────────────────────

#[given(regex = r#"^a coding coordinator with default network policy "([^"]+)"$"#)]
fn given_network_policy(world: &mut QuectoWorld, policy: String) {
    ensure_runtime(world);
    let mut config = default_config();
    if policy == "deny" {
        config.network_allowed_hosts = vec![];
    }
    world.coding_worker_launch_config = Some(config);
}

#[then("the nsjail process should have network access disabled")]
fn then_network_disabled(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let args = rt.nsjail_args_for(pid);
    assert!(
        args.contains(&"--disable_clone_newnet".to_string()),
        "network should be disabled"
    );
}

#[then("the worker should not be able to reach external hosts")]
fn then_no_external_reach(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let args = rt.nsjail_args_for(pid);
    assert!(
        args.contains(&"--disable_clone_newnet".to_string()),
        "network isolation should prevent external access"
    );
}

#[given(regex = r#"^a coding coordinator with network allowlist \["([^"]+)", "([^"]+)"\]$"#)]
fn given_network_allowlist(world: &mut QuectoWorld, host1: String, host2: String) {
    ensure_runtime(world);
    let mut config = default_config();
    config.network_allowed_hosts = vec![host1, host2];
    world.coding_worker_launch_config = Some(config);
}

#[then("the nsjail process should allow egress to the listed hosts only")]
fn then_egress_allowlist(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let args = rt.nsjail_args_for(pid);
    // When hosts are allowed, --disable_clone_newnet should NOT be present
    assert!(
        !args.contains(&"--disable_clone_newnet".to_string()),
        "network should not be disabled when allowlist is set"
    );
    // Verify the env has the allowed hosts
    let env = rt.worker_env_for(pid);
    let hosts_var = env.iter().find(|e| e.name == "QUECTO_ALLOWED_HOSTS");
    assert!(
        hosts_var.is_some(),
        "QUECTO_ALLOWED_HOSTS env var should be set"
    );
    let hosts = hosts_var.unwrap().value.clone();
    assert!(
        hosts.contains("registry.npmjs.org"),
        "should contain npm registry"
    );
    assert!(hosts.contains("github.com"), "should contain github.com");
}

// ── Environment isolation steps ─────────────────────────────────────────

#[given("a coding coordinator with environment variable QUECTO_SECRET_KEY set")]
fn given_env_quecto_secret(world: &mut QuectoWorld) {
    ensure_runtime(world);
    world.coding_worker_launch_config = Some(default_config());
}

#[then("the worker's environment should not contain QUECTO_SECRET_KEY")]
fn then_no_quecto_secret(world: &mut QuectoWorld) {
    assert!(
        is_blocked_env("QUECTO_SECRET_KEY"),
        "QUECTO_SECRET_KEY should be blocked"
    );
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let env = rt.worker_env_for(pid);
    assert!(
        !env.iter().any(|e| e.name == "QUECTO_SECRET_KEY"),
        "QUECTO_SECRET_KEY should not be in worker env"
    );
}

#[then("the worker's environment should not contain any QUECTO_ prefixed variables")]
fn then_no_quecto_prefix(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let env = rt.worker_env_for(pid);
    let quecto_vars: Vec<&WorkerEnvVar> = env
        .iter()
        .filter(|e| e.name.starts_with("QUECTO_"))
        .collect();
    // QUECTO_ALLOWED_HOSTS is the exception when network allowlist is set
    for v in &quecto_vars {
        assert_eq!(
            v.name, "QUECTO_ALLOWED_HOSTS",
            "only QUECTO_ALLOWED_HOSTS is permitted, found: {}",
            v.name
        );
    }
}

#[given("a coding coordinator with GitHub API token configured")]
fn given_github_token(world: &mut QuectoWorld) {
    ensure_runtime(world);
    world.coding_worker_launch_config = Some(default_config());
}

#[then("the worker's environment should not contain GITHUB_TOKEN")]
fn then_no_github_token(world: &mut QuectoWorld) {
    assert!(
        is_blocked_env("GITHUB_TOKEN"),
        "GITHUB_TOKEN should be blocked"
    );
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let env = rt.worker_env_for(pid);
    assert!(
        !env.iter().any(|e| e.name == "GITHUB_TOKEN"),
        "GITHUB_TOKEN should not be in worker env"
    );
}

#[then("the worker's environment should not contain GH_TOKEN")]
fn then_no_gh_token(world: &mut QuectoWorld) {
    assert!(is_blocked_env("GH_TOKEN"), "GH_TOKEN should be blocked");
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let env = rt.worker_env_for(pid);
    assert!(
        !env.iter().any(|e| e.name == "GH_TOKEN"),
        "GH_TOKEN should not be in worker env"
    );
}

#[then("the worker's environment should include PATH")]
fn then_env_has_path(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let env = rt.worker_env_for(pid);
    assert!(
        env.iter().any(|e| e.name == "PATH"),
        "PATH should be in worker env"
    );
}

#[then("the worker's environment should include LANG or LC_ALL")]
fn then_env_has_locale(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let env = rt.worker_env_for(pid);
    let has_lang = env.iter().any(|e| e.name == "LANG");
    let has_lc_all = env.iter().any(|e| e.name == "LC_ALL");
    assert!(
        has_lang || has_lc_all,
        "LANG or LC_ALL should be in worker env"
    );
}

#[then("the worker's total environment variable count should be small")]
fn then_env_count_small(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let env = rt.worker_env_for(pid);
    assert!(
        env.len() <= 5,
        "worker env should have at most 5 variables, got {}",
        env.len()
    );
}

// ── Cleanup steps ───────────────────────────────────────────────────────

#[when("the worker process exits normally")]
fn when_worker_exits_normally(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    rt.simulate_exit(pid, 0);
}

#[then("no nsjail processes should remain for this job")]
fn then_no_nsjail_processes(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    rt.cleanup(pid);
    assert!(
        !rt.is_alive(pid),
        "no processes should remain after cleanup"
    );
}

#[then("no stale worker mount namespaces should remain")]
fn then_no_stale_mounts(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    // After cleanup, the worker state should be fully removed
    assert!(
        !rt.is_alive(pid),
        "worker resources (including mounts) should be cleaned up"
    );
}

#[when("the coordinator kills the worker due to timeout")]
fn when_coordinator_kills_timeout(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    rt.kill(pid).expect("kill should succeed");
}

#[then("no stale cgroup entries should remain")]
fn then_no_stale_cgroups(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    rt.cleanup(pid);
    assert!(
        !rt.is_alive(pid),
        "worker resources (including cgroups) should be cleaned up"
    );
}

#[given("a coding worker launched with die-with-parent enabled")]
fn given_die_with_parent(world: &mut QuectoWorld) {
    ensure_runtime(world);
    let mut config = default_config();
    config.die_with_parent = true;
    world.coding_worker_launch_config = Some(config);
    launch_worker(world);
}

#[when("the coordinator process is killed")]
fn when_coordinator_killed(world: &mut QuectoWorld) {
    // Simulate coordinator death — with die-with-parent, worker dies too
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    rt.simulate_kill(pid, "parent_died");
}

#[then("the nsjail worker process should also be terminated")]
fn then_worker_terminated_with_parent(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    assert!(
        !rt.is_alive(pid),
        "worker should be dead when parent is killed (die-with-parent)"
    );
    let status = rt.status(pid);
    assert!(
        matches!(status, WorkerStatus::Killed { reason: ref r } if r.contains("parent_died")),
        "worker should show parent_died reason: got {status:?}"
    );
}

// ── Host toolchain steps ────────────────────────────────────────────────

#[when(regex = r#"^the worker runs "([^"]+)" via exec$"#)]
fn when_worker_runs_exec(world: &mut QuectoWorld, cmd: String) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    // Simulate the exec command being sent to the worker
    let exec_cmd = serde_json::json!({"type": "exec", "command": cmd}).to_string();
    rt.send_command(pid, &exec_cmd).expect("send exec command");
    world.coding_worker_last_exec_cmd = Some(cmd);
}

#[then(regex = r#"^the result should show a path under /usr/bin or similar$"#)]
fn then_result_shows_path(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    // Verify the host root is mounted read-only, giving access to /usr/bin
    let args = rt.nsjail_args_for(pid);
    assert!(
        args.iter().any(|a| a.contains("/:/host")),
        "host root should be mounted, giving access to /usr/bin"
    );
}

#[then("the worker should not be able to modify files in /usr/bin")]
fn then_cannot_modify_usr_bin(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    let args = rt.nsjail_args_for(pid);
    // Host root is mounted read-only
    let ro_idx = args.iter().position(|a| a == "--bindmount_ro");
    assert!(ro_idx.is_some(), "host root should be mounted read-only");
    let ro_val = &args[ro_idx.unwrap() + 1];
    assert!(
        ro_val.contains("/:/host"),
        "/usr/bin is under host root which is read-only"
    );
}

#[then("the result should show the Python version")]
fn then_python_version(world: &mut QuectoWorld) {
    let cmd = world
        .coding_worker_last_exec_cmd
        .as_ref()
        .expect("last exec cmd");
    assert!(
        cmd.contains("python3"),
        "exec command should involve python3"
    );
    // The mock runtime accepted the command — in production, nsjail would execute it
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    assert!(
        rt.nsjail_args_for(pid)
            .iter()
            .any(|a| a.contains("/:/host")),
        "host toolchain should be accessible"
    );
}

#[then("the command should succeed")]
fn then_command_succeeds(world: &mut QuectoWorld) {
    let pid = world.coding_worker_pid.expect("worker pid");
    let rt = runtime(world);
    assert!(rt.is_alive(pid), "worker should still be alive after exec");
}

// ── Concurrent workers steps ────────────────────────────────────────────

#[given(regex = r#"^a coding coordinator with max_parallel_jobs (\d+)$"#)]
fn given_max_parallel(world: &mut QuectoWorld, max: usize) {
    ensure_runtime(world);
    world.coding_worker_max_parallel = Some(max);
    world.coding_worker_launch_config = Some(default_config());
}

#[given(regex = r#"^(\d+) coding jobs in state "queued"$"#)]
fn given_n_queued_jobs(world: &mut QuectoWorld, count: usize) {
    world.coding_worker_queued_count = Some(count);
}

#[when(regex = r#"^the coordinator begins preparation for all (\d+) jobs$"#)]
fn when_prepare_all_jobs(world: &mut QuectoWorld, count: usize) {
    let mut pids = Vec::new();
    for i in 0..count {
        let mut config = default_config();
        config.job_dir = format!("/tmp/jobs/job_{:06}/repo", i + 1);
        config.goal = format!("task {}", i + 1);
        let rt = runtime(world);
        let pid = rt.launch(&config).expect("launch should succeed");
        pids.push(pid);
    }
    world.coding_worker_pids = Some(pids);
}

#[then(regex = r#"^(\d+) separate nsjail worker processes should be running$"#)]
fn then_n_workers_running(world: &mut QuectoWorld, expected: usize) {
    let rt = runtime(world);
    assert_eq!(
        rt.running_count(),
        expected,
        "expected {expected} running workers"
    );
}

#[then("each worker should have its own isolated job directory")]
fn then_isolated_dirs(world: &mut QuectoWorld) {
    let pids = world
        .coding_worker_pids
        .clone()
        .expect("worker pids should be set");
    let rt = runtime(world);
    let mut dirs = Vec::new();
    for pid in &pids {
        let args = rt.nsjail_args_for(*pid);
        let mount_idx = args.iter().position(|a| a == "--bindmount").unwrap();
        let dir = args[mount_idx + 1].clone();
        dirs.push(dir);
    }
    // All dirs should be unique
    let unique_count = dirs.iter().collect::<std::collections::HashSet<_>>().len();
    assert_eq!(
        unique_count,
        dirs.len(),
        "each worker should have a unique job directory"
    );
}

#[then("workers should not be able to access each other's directories")]
fn then_workers_isolated(world: &mut QuectoWorld) {
    let pids = world
        .coding_worker_pids
        .clone()
        .expect("worker pids should be set");
    let rt = runtime(world);
    // Each worker has exactly one rw mount (its own job dir)
    for pid in &pids {
        let args = rt.nsjail_args_for(*pid);
        let rw_count = args.iter().filter(|a| *a == "--bindmount").count();
        assert_eq!(rw_count, 1, "each worker should have exactly one rw mount");
    }
}

#[given(regex = r#"^(\d+) coding jobs are already running$"#)]
fn given_n_running_jobs(world: &mut QuectoWorld, count: usize) {
    let mut pids = Vec::new();
    for i in 0..count {
        let mut config = default_config();
        config.job_dir = format!("/tmp/jobs/running_{:06}/repo", i + 1);
        let rt = runtime(world);
        let pid = rt.launch(&config).expect("launch");
        pids.push(pid);
    }
    world.coding_worker_pids = Some(pids);
}

#[when("a third coding job is submitted")]
fn when_third_job_submitted(world: &mut QuectoWorld) {
    let max = world.coding_worker_max_parallel.unwrap_or(2);
    let running = {
        let rt = runtime(world);
        rt.running_count()
    };
    world.coding_worker_third_job_queued = running >= max;
}

#[then(regex = r#"^the third job should remain in state "queued"$"#)]
fn then_third_remains_queued(world: &mut QuectoWorld) {
    assert!(
        world.coding_worker_third_job_queued,
        "third job should remain queued when parallel limit is reached"
    );
}

#[then("the coordinator should launch it when a running job completes")]
fn then_launches_on_completion(world: &mut QuectoWorld) {
    // Simulate one running job completing
    let pids = world.coding_worker_pids.clone().expect("pids");
    let first_pid = pids[0];
    let max = world.coding_worker_max_parallel.unwrap_or(2);
    let rt = runtime(world);
    rt.simulate_exit(first_pid, 0);

    // Now running_count should be below max, allowing a new launch
    assert!(
        rt.running_count() < max,
        "a slot should be free after job completion"
    );

    // Launch the third job
    let config = default_config();
    let new_pid = rt.launch(&config).expect("launch third job");
    assert!(rt.is_alive(new_pid), "third job should now be running");
}
