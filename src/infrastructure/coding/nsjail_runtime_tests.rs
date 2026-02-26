use super::*;

fn default_launch_config() -> WorkerLaunchConfig {
    WorkerLaunchConfig {
        run_id: "run_001".to_string(),
        job_id: "job_001".to_string(),
        job_dir: "/tmp/jobs/job_001/repo".to_string(),
        goal: "fix tests".to_string(),
        max_memory_mb: 512,
        max_cpu_seconds: 120,
        max_wall_seconds: 300,
        max_pids: 128,
        network_allowed_hosts: vec![],
        die_with_parent: true,
    }
}

fn test_config() -> NsjailRuntimeConfig {
    NsjailRuntimeConfig {
        nsjail_binary: "nsjail".to_string(),
        quecto_binary: "/usr/local/bin/quecto".to_string(),
        command_override: None,
        cgroups_available: true,
    }
}

#[test]
fn test_build_nsjail_args_contains_mode() {
    let args = build_nsjail_worker_args(&test_config(), &default_launch_config(), "r1", "j1");
    assert!(args.nsjail_args.contains(&"--mode".to_string()));
    assert!(args.nsjail_args.contains(&"o".to_string()));
}

#[test]
fn test_build_nsjail_args_contains_separator() {
    let args = build_nsjail_worker_args(&test_config(), &default_launch_config(), "r1", "j1");
    assert!(args.nsjail_args.contains(&"--".to_string()));
}

#[test]
fn test_build_nsjail_args_worker_starts_with_quecto() {
    let args = build_nsjail_worker_args(&test_config(), &default_launch_config(), "r1", "j1");
    assert_eq!(
        args.worker_args[0], "/usr/local/bin/quecto",
        "first worker arg should be quecto binary"
    );
    assert_eq!(args.worker_args[1], "worker");
}

#[test]
fn test_build_full_command_includes_run_and_job_id() {
    let full =
        build_full_nsjail_command(&test_config(), &default_launch_config(), "run-42", "job-99");
    let run_idx = full.iter().position(|a| a == "--run-id").unwrap();
    assert_eq!(full[run_idx + 1], "run-42");
    let job_idx = full.iter().position(|a| a == "--job-id").unwrap();
    assert_eq!(full[job_idx + 1], "job-99");
}

#[test]
fn test_build_full_command_includes_goal() {
    let full = build_full_nsjail_command(&test_config(), &default_launch_config(), "r1", "j1");
    let goal_idx = full.iter().position(|a| a == "--goal").unwrap();
    assert_eq!(full[goal_idx + 1], "fix tests");
}

#[test]
fn test_mount_job_dir_rw() {
    let config = default_launch_config();
    let args = build_nsjail_worker_args(&test_config(), &config, "r1", "j1");
    let bind_idx = args
        .nsjail_args
        .iter()
        .position(|a| a == "--bindmount")
        .unwrap();
    assert!(args.nsjail_args[bind_idx + 1].contains(&config.job_dir));
}

#[test]
fn test_mount_host_root_ro() {
    let args = build_nsjail_worker_args(&test_config(), &default_launch_config(), "r1", "j1");
    let ro_idx = args
        .nsjail_args
        .iter()
        .position(|a| a == "--bindmount_ro")
        .unwrap();
    assert_eq!(args.nsjail_args[ro_idx + 1], "/:/host");
}

#[test]
fn test_resource_limits() {
    let mut config = default_launch_config();
    config.max_memory_mb = 1024;
    config.max_cpu_seconds = 180;
    config.max_wall_seconds = 600;
    config.max_pids = 256;
    let args = build_nsjail_worker_args(&test_config(), &config, "r1", "j1");
    let all = &args.nsjail_args;

    let mem_idx = all.iter().position(|a| a == "--rlimit_as").unwrap();
    assert_eq!(all[mem_idx + 1], "1024");

    let cpu_idx = all.iter().position(|a| a == "--rlimit_cpu").unwrap();
    assert_eq!(all[cpu_idx + 1], "180");

    let wall_idx = all.iter().position(|a| a == "--time_limit").unwrap();
    assert_eq!(all[wall_idx + 1], "600");

    let pid_idx = all.iter().position(|a| a == "--cgroup_pids_max").unwrap();
    assert_eq!(all[pid_idx + 1], "256");
}

#[test]
fn test_security_flags() {
    let args = build_nsjail_worker_args(&test_config(), &default_launch_config(), "r1", "j1");
    assert!(args.nsjail_args.contains(&"--no_new_privs".to_string()));
    assert!(args.nsjail_args.contains(&"--seccomp_string".to_string()));
}

#[test]
fn test_network_disabled_by_default() {
    let args = build_nsjail_worker_args(&test_config(), &default_launch_config(), "r1", "j1");
    assert!(
        args.nsjail_args
            .contains(&"--disable_clone_newnet".to_string())
    );
}

#[test]
fn test_network_enabled_with_hosts() {
    let mut config = default_launch_config();
    config.network_allowed_hosts = vec!["github.com".into()];
    let args = build_nsjail_worker_args(&test_config(), &config, "r1", "j1");
    assert!(
        !args
            .nsjail_args
            .contains(&"--disable_clone_newnet".to_string())
    );
}

#[test]
fn test_die_with_parent_enabled() {
    let args = build_nsjail_worker_args(&test_config(), &default_launch_config(), "r1", "j1");
    assert!(args.nsjail_args.contains(&"--die_with_parent".to_string()));
}

#[test]
fn test_die_with_parent_disabled() {
    let mut config = default_launch_config();
    config.die_with_parent = false;
    let args = build_nsjail_worker_args(&test_config(), &config, "r1", "j1");
    assert!(!args.nsjail_args.contains(&"--die_with_parent".to_string()));
}

#[test]
fn test_worker_env_minimal() {
    let env = build_worker_env(&default_launch_config());
    let names: Vec<&str> = env.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"PATH"));
    assert!(names.contains(&"LANG"));
    assert!(names.contains(&"HOME"));
}

#[test]
fn test_worker_env_lang_value() {
    let env = build_worker_env(&default_launch_config());
    let lang = env.iter().find(|e| e.name == "LANG").unwrap();
    assert_eq!(lang.value, "C.UTF-8");
}

#[test]
fn test_worker_env_home_is_job_dir() {
    let config = default_launch_config();
    let env = build_worker_env(&config);
    let home = env.iter().find(|e| e.name == "HOME").unwrap();
    assert_eq!(home.value, config.job_dir);
}

#[test]
fn test_worker_env_allowed_hosts() {
    let mut config = default_launch_config();
    config.network_allowed_hosts = vec!["github.com".into(), "npm.org".into()];
    let env = build_worker_env(&config);
    let hosts = env
        .iter()
        .find(|e| e.name == "QUECTO_ALLOWED_HOSTS")
        .unwrap();
    assert_eq!(hosts.value, "github.com,npm.org");
}

#[test]
fn test_blocked_env_vars() {
    assert!(is_blocked_env("QUECTO_SECRET"));
    assert!(is_blocked_env("GITHUB_TOKEN"));
    assert!(is_blocked_env("GH_TOKEN"));
    assert!(is_blocked_env("OPENAI_API_KEY"));
    assert!(is_blocked_env("ANTHROPIC_API_KEY"));
    assert!(!is_blocked_env("PATH"));
    assert!(!is_blocked_env("HOME"));
}

#[test]
fn test_resolve_quecto_binary() {
    let path = resolve_quecto_binary();
    assert!(!path.is_empty());
}

#[test]
fn test_quiet_mode() {
    let args = build_nsjail_worker_args(&test_config(), &default_launch_config(), "r1", "j1");
    assert!(args.nsjail_args.contains(&"--quiet".to_string()));
}

#[test]
fn test_cwd_is_job_dir() {
    let config = default_launch_config();
    let args = build_nsjail_worker_args(&test_config(), &config, "r1", "j1");
    let cwd_idx = args.nsjail_args.iter().position(|a| a == "--cwd").unwrap();
    assert_eq!(args.nsjail_args[cwd_idx + 1], config.job_dir);
}

#[test]
fn test_runtime_launch_and_status() {
    let mut rt = NsjailWorkerRuntime::new(test_config());
    let config = default_launch_config();
    let pid = rt.launch(&config).unwrap();
    assert_eq!(rt.status(pid), WorkerStatus::Running);
    assert!(rt.is_alive(pid));
}

#[test]
fn test_runtime_unknown_pid_status() {
    let rt = NsjailWorkerRuntime::new(test_config());
    let status = rt.status(99999);
    assert!(matches!(status, WorkerStatus::Killed { reason } if reason.contains("unknown")));
}

#[test]
fn test_runtime_unknown_pid_not_alive() {
    let rt = NsjailWorkerRuntime::new(test_config());
    assert!(!rt.is_alive(99999));
}

#[test]
fn test_runtime_stderr_cap() {
    let mut rt = NsjailWorkerRuntime::new(test_config());
    let config = default_launch_config();
    let pid = rt.launch(&config).unwrap();

    // Inject more than 1 MiB
    let big = "x".repeat(MAX_STDERR_BYTES + 1000);
    rt.inject_stderr(pid, &big);
    let captured = rt.read_stderr(pid);
    assert_eq!(captured.len(), MAX_STDERR_BYTES);
}

#[test]
fn test_runtime_cleanup() {
    let mut rt = NsjailWorkerRuntime::new(test_config());
    let config = default_launch_config();
    let pid = rt.launch(&config).unwrap();
    rt.cleanup(pid);
    assert!(!rt.is_alive(pid));
}

#[test]
fn test_runtime_kill() {
    let mut rt = NsjailWorkerRuntime::new(test_config());
    let config = default_launch_config();
    let pid = rt.launch(&config).unwrap();
    rt.kill(pid).unwrap();
    assert!(!rt.is_alive(pid));
}

#[test]
fn test_launch_params_tracking() {
    let mut rt = NsjailWorkerRuntime::new(test_config());
    let config = default_launch_config();
    rt.launch(&config).unwrap();
    let (run_id, job_id) = rt.last_launch_params().unwrap();
    assert!(run_id.starts_with("run_"));
    assert!(job_id.starts_with("job_"));
}

#[test]
fn test_validate_job_dir_rejects_colon() {
    let result = validate_job_dir_for_nsjail("/tmp/jobs:evil/repo");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("unsafe character"));
}

#[test]
fn test_validate_job_dir_rejects_newline() {
    let result = validate_job_dir_for_nsjail("/tmp/jobs\n/repo");
    assert!(result.is_err());
}

#[test]
fn test_validate_job_dir_rejects_null() {
    let result = validate_job_dir_for_nsjail("/tmp/jobs\0/repo");
    assert!(result.is_err());
}

#[test]
fn test_validate_job_dir_accepts_clean_path() {
    assert!(validate_job_dir_for_nsjail("/tmp/jobs/job_001/repo").is_ok());
}

#[test]
fn test_launch_rejects_unsafe_job_dir() {
    let mut rt = NsjailWorkerRuntime::new(test_config());
    let mut config = default_launch_config();
    config.job_dir = "/tmp/jobs:evil/repo".to_string();
    let result = rt.launch(&config);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("unsafe character"));
}

#[test]
fn test_floor_char_boundary_ascii() {
    assert_eq!(floor_char_boundary("hello", 3), 3);
}

#[test]
fn test_floor_char_boundary_multibyte() {
    // "é" is 2 bytes in UTF-8 (0xC3 0xA9)
    let s = "café";
    // "caf" is 3 bytes, "é" starts at byte 3
    // Requesting byte 4 lands in the middle of "é"
    assert_eq!(floor_char_boundary(s, 4), 3);
    // Requesting byte 5 lands exactly after "é"
    assert_eq!(floor_char_boundary(s, 5), 5);
}

#[test]
fn test_floor_char_boundary_at_end() {
    let s = "hello";
    assert_eq!(floor_char_boundary(s, 100), 5);
}

#[test]
fn test_inject_stderr_utf8_safe() {
    let mut rt = NsjailWorkerRuntime::new(test_config());
    let config = default_launch_config();
    let pid = rt.launch(&config).unwrap();
    // Inject a string with multi-byte chars near the cut point
    let data = "a".repeat(MAX_STDERR_BYTES - 1) + "é"; // é is 2 bytes
    rt.inject_stderr(pid, &data);
    let captured = rt.read_stderr(pid);
    // Should be valid UTF-8 and not exceed limit
    assert!(captured.len() <= MAX_STDERR_BYTES);
    assert!(captured.is_char_boundary(captured.len()));
}
