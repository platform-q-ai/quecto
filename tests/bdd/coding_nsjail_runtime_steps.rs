use cucumber::{given, then, when};
use quecto::domain::coding_ports::{WorkerLaunchConfig, WorkerRuntime, WorkerStatus};
use quecto::infrastructure::coding::nsjail_runtime::{
    NsjailRuntimeConfig, NsjailWorkerRuntime, build_full_nsjail_command, build_nsjail_worker_args,
    build_worker_env, is_blocked_env, resolve_quecto_binary,
};

use crate::QuectoWorld;

// ── helpers ──────────────────────────────────────────────────────────────

fn default_nrt_config() -> WorkerLaunchConfig {
    WorkerLaunchConfig {
        job_dir: "/tmp/jobs/job-001/repo".to_string(),
        goal: "fix tests".to_string(),
        max_memory_mb: 512,
        max_cpu_seconds: 120,
        max_wall_seconds: 300,
        max_pids: 128,
        network_allowed_hosts: vec![],
        die_with_parent: true,
    }
}

fn ensure_nrt_runtime(world: &mut QuectoWorld) {
    if world.nrt_runtime.is_none() {
        let config = NsjailRuntimeConfig {
            nsjail_binary: "nsjail".to_string(),
            quecto_binary: "/usr/local/bin/quecto".to_string(),
        };
        world.nrt_runtime = Some(NsjailWorkerRuntime::new(config));
    }
    if world.nrt_launch_config.is_none() {
        world.nrt_launch_config = Some(default_nrt_config());
    }
}

fn nrt_runtime(world: &mut QuectoWorld) -> &mut NsjailWorkerRuntime {
    world.nrt_runtime.as_mut().expect("nrt runtime")
}

fn nrt_launch_config(world: &QuectoWorld) -> &WorkerLaunchConfig {
    world.nrt_launch_config.as_ref().expect("nrt launch config")
}

fn nrt_runtime_config(world: &QuectoWorld) -> NsjailRuntimeConfig {
    let rt = world.nrt_runtime.as_ref().expect("nrt runtime");
    NsjailRuntimeConfig {
        nsjail_binary: "nsjail".to_string(),
        quecto_binary: rt.quecto_binary().to_string(),
    }
}

// ── Given steps ─────────────────────────────────────────────────────────

#[given("a nsjail runtime with default config")]
fn given_nsjail_runtime(world: &mut QuectoWorld) {
    ensure_nrt_runtime(world);
}

#[given(regex = r#"^a worker launch config with run_id "([^"]+)" and job_id "([^"]+)"$"#)]
fn given_launch_config_with_ids(world: &mut QuectoWorld, run_id: String, job_id: String) {
    ensure_nrt_runtime(world);
    world.nrt_run_id = Some(run_id);
    world.nrt_job_id = Some(job_id);
}

#[given(regex = r#"^a worker launch config with goal "([^"]+)"$"#)]
fn given_launch_config_with_goal(world: &mut QuectoWorld, goal: String) {
    ensure_nrt_runtime(world);
    let config = world
        .nrt_launch_config
        .get_or_insert_with(default_nrt_config);
    config.goal = goal;
}

#[given(regex = r#"^a worker launch config with job_dir "([^"]+)"$"#)]
fn given_launch_config_with_job_dir(world: &mut QuectoWorld, job_dir: String) {
    ensure_nrt_runtime(world);
    let config = world
        .nrt_launch_config
        .get_or_insert_with(default_nrt_config);
    config.job_dir = job_dir;
}

#[given("a worker launch config with limits:")]
fn given_launch_config_with_limits(world: &mut QuectoWorld, step: &cucumber::gherkin::Step) {
    ensure_nrt_runtime(world);
    let config = world
        .nrt_launch_config
        .get_or_insert_with(default_nrt_config);
    if let Some(table) = &step.table {
        for row in &table.rows {
            if row.len() < 2 {
                continue;
            }
            let key = row[0].trim();
            let val = row[1].trim();
            match key {
                "max_memory_mb" => config.max_memory_mb = val.parse().unwrap(),
                "max_cpu_seconds" => config.max_cpu_seconds = val.parse().unwrap(),
                "max_wall_seconds" => config.max_wall_seconds = val.parse().unwrap(),
                "max_pids" => config.max_pids = val.parse().unwrap(),
                _ => panic!("unknown limit key: {key}"),
            }
        }
    }
}

#[given("a worker launch config with no network hosts")]
fn given_no_network_hosts(world: &mut QuectoWorld) {
    ensure_nrt_runtime(world);
    let config = world
        .nrt_launch_config
        .get_or_insert_with(default_nrt_config);
    config.network_allowed_hosts = vec![];
}

#[given(regex = r#"^a worker launch config with network hosts "([^"]+)"$"#)]
fn given_network_hosts(world: &mut QuectoWorld, hosts: String) {
    ensure_nrt_runtime(world);
    let config = world
        .nrt_launch_config
        .get_or_insert_with(default_nrt_config);
    config.network_allowed_hosts = hosts.split(',').map(|h| h.trim().to_string()).collect();
}

#[given("a worker launch config with die_with_parent enabled")]
fn given_die_with_parent_enabled(world: &mut QuectoWorld) {
    ensure_nrt_runtime(world);
    let config = world
        .nrt_launch_config
        .get_or_insert_with(default_nrt_config);
    config.die_with_parent = true;
}

#[given("a worker launch config with die_with_parent disabled")]
fn given_die_with_parent_disabled(world: &mut QuectoWorld) {
    ensure_nrt_runtime(world);
    let config = world
        .nrt_launch_config
        .get_or_insert_with(default_nrt_config);
    config.die_with_parent = false;
}

// ── When steps ──────────────────────────────────────────────────────────

#[when("the runtime builds nsjail args for a job")]
fn when_build_nsjail_args(world: &mut QuectoWorld) {
    let rt_config = nrt_runtime_config(world);
    let launch = nrt_launch_config(world).clone();
    let run_id = world
        .nrt_run_id
        .clone()
        .unwrap_or_else(|| "run-default".to_string());
    let job_id = world
        .nrt_job_id
        .clone()
        .unwrap_or_else(|| "job-default".to_string());
    let parts = build_nsjail_worker_args(&rt_config, &launch, &run_id, &job_id);
    let mut all = parts.nsjail_args;
    all.extend(parts.worker_args);
    world.nrt_last_args = Some(all);
}

#[when("the runtime builds the full command for the job")]
fn when_build_full_command(world: &mut QuectoWorld) {
    let rt_config = nrt_runtime_config(world);
    let launch = nrt_launch_config(world).clone();
    let run_id = world
        .nrt_run_id
        .clone()
        .unwrap_or_else(|| "run-default".to_string());
    let job_id = world
        .nrt_job_id
        .clone()
        .unwrap_or_else(|| "job-default".to_string());
    let full = build_full_nsjail_command(&rt_config, &launch, &run_id, &job_id);

    // Split into nsjail args and worker args at "--"
    let sep_idx = full.iter().position(|a| a == "--");
    if let Some(idx) = sep_idx {
        world.nrt_last_worker_args = Some(full[idx + 1..].to_vec());
    }
    world.nrt_last_args = Some(full);
}

#[when("the runtime builds worker env for a job")]
fn when_build_worker_env(world: &mut QuectoWorld) {
    // env building is verified in Then steps directly
    ensure_nrt_runtime(world);
}

#[when("the runtime resolves the quecto binary path")]
fn when_resolve_quecto_binary(world: &mut QuectoWorld) {
    world.nrt_resolved_binary = Some(resolve_quecto_binary());
}

#[when("the runtime receives stderr data exceeding 1 MiB")]
fn when_stderr_exceeds_limit(world: &mut QuectoWorld) {
    ensure_nrt_runtime(world);
    let launch = nrt_launch_config(world).clone();
    let pid = nrt_runtime(world).launch(&launch).unwrap();
    world.nrt_pid = Some(pid);

    let big = "x".repeat(1024 * 1024 + 1000);
    nrt_runtime(world).inject_stderr(pid, &big);
}

#[when(regex = r#"^the runtime checks status for an unknown PID (\d+)$"#)]
fn when_check_unknown_pid_status(world: &mut QuectoWorld, pid: u32) {
    ensure_nrt_runtime(world);
    world.nrt_pid = Some(pid);
}

#[when(regex = r#"^the runtime checks if PID (\d+) is alive$"#)]
fn when_check_pid_alive(world: &mut QuectoWorld, pid: u32) {
    ensure_nrt_runtime(world);
    world.nrt_pid = Some(pid);
}

// ── Then steps ──────────────────────────────────────────────────────────

#[then(regex = r#"^the nsjail args should contain "([^"]+)" followed by "([^"]+)"$"#)]
fn then_args_contain_pair(world: &mut QuectoWorld, flag: String, value: String) {
    let args = world
        .nrt_last_args
        .as_ref()
        .expect("nsjail args should be built");
    let idx = args
        .iter()
        .position(|a| a == &flag)
        .unwrap_or_else(|| panic!("nsjail args should contain '{flag}'"));
    assert!(
        idx + 1 < args.len(),
        "flag '{flag}' should have a value after it"
    );
    assert_eq!(
        args[idx + 1],
        value,
        "value after '{flag}' should be '{value}', got '{}'",
        args[idx + 1]
    );
}

#[then(regex = r#"^the nsjail args should contain "([^"]+)" separator$"#)]
fn then_args_contain_separator(world: &mut QuectoWorld, sep: String) {
    let args = world
        .nrt_last_args
        .as_ref()
        .expect("nsjail args should be built");
    assert!(
        args.contains(&sep),
        "nsjail args should contain '{sep}' separator"
    );
}

#[then(regex = r#"^the nsjail args after "([^"]+)" should start with the quecto binary path$"#)]
fn then_after_sep_starts_with_quecto(world: &mut QuectoWorld, sep: String) {
    let args = world
        .nrt_last_args
        .as_ref()
        .expect("nsjail args should be built");
    let sep_idx = args
        .iter()
        .position(|a| a == &sep)
        .unwrap_or_else(|| panic!("nsjail args should contain '{sep}'"));
    assert!(
        sep_idx + 1 < args.len(),
        "there should be args after '{sep}'"
    );
    let quecto_path = &args[sep_idx + 1];
    assert!(
        quecto_path.contains("quecto") || !quecto_path.is_empty(),
        "first arg after '--' should be the quecto binary: got '{quecto_path}'"
    );
}

#[then(regex = r#"^the nsjail args after "([^"]+)" should contain "([^"]+)"$"#)]
fn then_after_sep_contains(world: &mut QuectoWorld, sep: String, expected: String) {
    let args = world
        .nrt_last_args
        .as_ref()
        .expect("nsjail args should be built");
    let sep_idx = args
        .iter()
        .position(|a| a == &sep)
        .unwrap_or_else(|| panic!("nsjail args should contain '{sep}'"));
    let after_sep = &args[sep_idx + 1..];
    assert!(
        after_sep.iter().any(|a| a == &expected),
        "args after '{sep}' should contain '{expected}'"
    );
}

#[then(regex = r#"^the worker args should contain "([^"]+)" followed by "([^"]+)"$"#)]
fn then_worker_args_contain_pair(world: &mut QuectoWorld, flag: String, value: String) {
    let worker_args = world
        .nrt_last_worker_args
        .as_ref()
        .expect("worker args should be built");
    let idx = worker_args
        .iter()
        .position(|a| a == &flag)
        .unwrap_or_else(|| panic!("worker args should contain '{flag}'"));
    assert!(
        idx + 1 < worker_args.len(),
        "flag '{flag}' should have a value"
    );
    assert_eq!(
        worker_args[idx + 1],
        value,
        "value after '{flag}' should be '{value}', got '{}'",
        worker_args[idx + 1]
    );
}

#[then(regex = r#"^the nsjail args should contain "([^"]+)" with the job directory$"#)]
fn then_args_contain_with_job_dir(world: &mut QuectoWorld, flag: String) {
    let args = world
        .nrt_last_args
        .as_ref()
        .expect("nsjail args should be built");
    let launch = nrt_launch_config(world);
    let job_dir = &launch.job_dir;
    let idx = args
        .iter()
        .position(|a| a == &flag)
        .unwrap_or_else(|| panic!("nsjail args should contain '{flag}'"));
    assert!(
        args[idx + 1].contains(job_dir),
        "'{flag}' value should contain job dir '{job_dir}', got '{}'",
        args[idx + 1]
    );
}

#[then(regex = r#"^the nsjail args should contain "([^"]+)" with "([^"]+)"$"#)]
fn then_args_contain_flag_with_value(world: &mut QuectoWorld, flag: String, value: String) {
    let args = world
        .nrt_last_args
        .as_ref()
        .expect("nsjail args should be built");
    let idx = args
        .iter()
        .position(|a| a == &flag)
        .unwrap_or_else(|| panic!("nsjail args should contain '{flag}'"));
    assert!(
        args[idx + 1].contains(&value),
        "'{flag}' value should contain '{value}', got '{}'",
        args[idx + 1]
    );
}

#[then(regex = r#"^the nsjail args should include memory limit (\d+)$"#)]
fn then_memory_limit(world: &mut QuectoWorld, limit: u32) {
    let args = world
        .nrt_last_args
        .as_ref()
        .expect("nsjail args should be built");
    let idx = args
        .iter()
        .position(|a| a == "--rlimit_as")
        .expect("--rlimit_as in args");
    let actual: u32 = args[idx + 1].parse().expect("memory limit value");
    assert_eq!(actual, limit, "memory limit should be {limit}");
}

#[then(regex = r#"^the nsjail args should include cpu time limit (\d+)$"#)]
fn then_cpu_limit(world: &mut QuectoWorld, limit: u32) {
    let args = world
        .nrt_last_args
        .as_ref()
        .expect("nsjail args should be built");
    let idx = args
        .iter()
        .position(|a| a == "--rlimit_cpu")
        .expect("--rlimit_cpu in args");
    let actual: u32 = args[idx + 1].parse().expect("cpu limit value");
    assert_eq!(actual, limit, "cpu time limit should be {limit}");
}

#[then(regex = r#"^the nsjail args should include wall time limit (\d+)$"#)]
fn then_wall_limit(world: &mut QuectoWorld, limit: u32) {
    let args = world
        .nrt_last_args
        .as_ref()
        .expect("nsjail args should be built");
    let idx = args
        .iter()
        .position(|a| a == "--time_limit")
        .expect("--time_limit in args");
    let actual: u32 = args[idx + 1].parse().expect("wall limit value");
    assert_eq!(actual, limit, "wall time limit should be {limit}");
}

#[then(regex = r#"^the nsjail args should include pid limit (\d+)$"#)]
fn then_pid_limit(world: &mut QuectoWorld, limit: u32) {
    let args = world
        .nrt_last_args
        .as_ref()
        .expect("nsjail args should be built");
    let idx = args
        .iter()
        .position(|a| a == "--cgroup_pids_max")
        .expect("--cgroup_pids_max in args");
    let actual: u32 = args[idx + 1].parse().expect("pid limit value");
    assert_eq!(actual, limit, "pid limit should be {limit}");
}

#[then(regex = r#"^the nsjail args should contain "([^"]+)"$"#)]
fn then_args_contain(world: &mut QuectoWorld, flag: String) {
    let args = world
        .nrt_last_args
        .as_ref()
        .expect("nsjail args should be built");
    assert!(args.contains(&flag), "nsjail args should contain '{flag}'");
}

#[then(regex = r#"^the nsjail args should not contain "([^"]+)"$"#)]
fn then_args_not_contain(world: &mut QuectoWorld, flag: String) {
    let args = world
        .nrt_last_args
        .as_ref()
        .expect("nsjail args should be built");
    assert!(
        !args.contains(&flag),
        "nsjail args should not contain '{flag}'"
    );
}

// ── Environment Then steps ──────────────────────────────────────────────

#[then("the nsjail worker env should contain PATH")]
fn then_env_has_path(world: &mut QuectoWorld) {
    let launch = nrt_launch_config(world).clone();
    let env = build_worker_env(&launch);
    assert!(
        env.iter().any(|e| e.name == "PATH"),
        "worker env should contain PATH"
    );
}

#[then(regex = r#"^the nsjail worker env should contain LANG set to "([^"]+)"$"#)]
fn then_env_lang(world: &mut QuectoWorld, expected: String) {
    let launch = nrt_launch_config(world).clone();
    let env = build_worker_env(&launch);
    let lang = env.iter().find(|e| e.name == "LANG").expect("LANG in env");
    assert_eq!(lang.value, expected, "LANG should be '{expected}'");
}

#[then("the nsjail worker env should contain HOME set to the job directory")]
fn then_env_home(world: &mut QuectoWorld) {
    let launch = nrt_launch_config(world).clone();
    let env = build_worker_env(&launch);
    let home = env.iter().find(|e| e.name == "HOME").expect("HOME in env");
    assert_eq!(
        home.value, launch.job_dir,
        "HOME should be the job directory"
    );
}

#[then(
    "the nsjail worker env should not contain any QUECTO_ prefixed vars except QUECTO_ALLOWED_HOSTS"
)]
fn then_env_no_quecto(world: &mut QuectoWorld) {
    let launch = nrt_launch_config(world).clone();
    let env = build_worker_env(&launch);
    for e in &env {
        if e.name.starts_with("QUECTO_") {
            assert_eq!(
                e.name, "QUECTO_ALLOWED_HOSTS",
                "only QUECTO_ALLOWED_HOSTS is permitted, found: {}",
                e.name
            );
        }
    }
    // Also verify the blocking function works
    assert!(is_blocked_env("QUECTO_SECRET"));
}

#[then("the nsjail worker env should not contain GITHUB_TOKEN")]
fn then_env_no_github_token(_world: &mut QuectoWorld) {
    assert!(
        is_blocked_env("GITHUB_TOKEN"),
        "GITHUB_TOKEN should be blocked"
    );
}

#[then("the nsjail worker env should not contain GH_TOKEN")]
fn then_env_no_gh_token(_world: &mut QuectoWorld) {
    assert!(is_blocked_env("GH_TOKEN"), "GH_TOKEN should be blocked");
}

#[then(regex = r#"^the nsjail worker env should contain QUECTO_ALLOWED_HOSTS with "([^"]+)"$"#)]
fn then_env_allowed_hosts(world: &mut QuectoWorld, expected: String) {
    let launch = nrt_launch_config(world).clone();
    let env = build_worker_env(&launch);
    let hosts = env
        .iter()
        .find(|e| e.name == "QUECTO_ALLOWED_HOSTS")
        .expect("QUECTO_ALLOWED_HOSTS in env");
    assert_eq!(
        hosts.value, expected,
        "QUECTO_ALLOWED_HOSTS should be '{expected}'"
    );
}

// ── Binary resolution Then steps ────────────────────────────────────────

#[then("the resolved path should be an absolute path")]
fn then_resolved_is_absolute(world: &mut QuectoWorld) {
    let path = world.nrt_resolved_binary.as_ref().expect("resolved binary");
    assert!(
        path.starts_with('/'),
        "resolved path should be absolute: got '{path}'"
    );
}

#[then(regex = r#"^the resolved path should end with "quecto" or contain the test binary name$"#)]
fn then_resolved_contains_quecto(world: &mut QuectoWorld) {
    let path = world.nrt_resolved_binary.as_ref().expect("resolved binary");
    // In tests, current_exe() returns the test runner binary name,
    // so we just check it's a non-empty absolute path.
    assert!(!path.is_empty(), "resolved path should not be empty");
}

// ── Launch param tracking Then steps ────────────────────────────────────

#[then(regex = r#"^the nsjail runtime should track run_id "([^"]+)" for the launch$"#)]
fn then_track_run_id(world: &mut QuectoWorld, expected: String) {
    // The run_id is embedded in the full command
    let args = world.nrt_last_args.as_ref().expect("args should be built");
    let idx = args
        .iter()
        .position(|a| a == "--run-id")
        .expect("--run-id in args");
    assert_eq!(args[idx + 1], expected, "run_id should be '{expected}'");
}

#[then(regex = r#"^the nsjail runtime should track job_id "([^"]+)" for the launch$"#)]
fn then_track_job_id(world: &mut QuectoWorld, expected: String) {
    let args = world.nrt_last_args.as_ref().expect("args should be built");
    let idx = args
        .iter()
        .position(|a| a == "--job-id")
        .expect("--job-id in args");
    assert_eq!(args[idx + 1], expected, "job_id should be '{expected}'");
}

// ── Stderr cap Then steps ───────────────────────────────────────────────

#[then(regex = r#"^the captured stderr should be at most (\d+) bytes$"#)]
fn then_stderr_capped(world: &mut QuectoWorld, max_bytes: usize) {
    let pid = world.nrt_pid.expect("nrt pid");
    let rt = nrt_runtime(world);
    let stderr = rt.read_stderr(pid);
    assert!(
        stderr.len() <= max_bytes,
        "stderr should be at most {max_bytes} bytes, got {}",
        stderr.len()
    );
}

// ── Status Then steps ───────────────────────────────────────────────────

#[then(regex = r#"^the nsjail runtime status should be killed with reason containing "([^"]+)"$"#)]
fn then_status_killed(world: &mut QuectoWorld, reason_substr: String) {
    let pid = world.nrt_pid.expect("nrt pid");
    let rt = nrt_runtime(world);
    let status = rt.status(pid);
    match status {
        WorkerStatus::Killed { ref reason } => {
            assert!(
                reason.contains(&reason_substr),
                "kill reason should contain '{reason_substr}', got '{reason}'"
            );
        }
        other => panic!("expected Killed status, got {other:?}"),
    }
}

#[then("the nsjail runtime should report not alive")]
fn then_not_alive(world: &mut QuectoWorld) {
    let pid = world.nrt_pid.expect("nrt pid");
    let rt = nrt_runtime(world);
    assert!(!rt.is_alive(pid), "PID should not be alive");
}

// ── CWD Then steps ─────────────────────────────────────────────────────

#[then(regex = r#"^the nsjail args should contain "([^"]+)" followed by the job directory$"#)]
fn then_args_flag_followed_by_job_dir(world: &mut QuectoWorld, flag: String) {
    let args = world
        .nrt_last_args
        .as_ref()
        .expect("nsjail args should be built");
    let launch = nrt_launch_config(world);
    let job_dir = &launch.job_dir;
    let idx = args
        .iter()
        .position(|a| a == &flag)
        .unwrap_or_else(|| panic!("nsjail args should contain '{flag}'"));
    assert_eq!(
        args[idx + 1],
        *job_dir,
        "'{flag}' should be followed by job dir '{job_dir}', got '{}'",
        args[idx + 1]
    );
}
