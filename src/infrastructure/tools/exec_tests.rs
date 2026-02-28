use super::*;
use tempfile::TempDir;

fn test_exec(restrict: bool) -> (ExecTool, TempDir) {
    let tmp = TempDir::new().unwrap();
    let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), restrict);
    let tool = ExecTool::new(Arc::new(tmp.path().to_path_buf()), Arc::new(sandbox));
    (tool, tmp)
}

/// Build an nsjail command with the given options and return the args as a joined string.
fn nsjail_args_str(options: &NsjailOptions) -> String {
    let workspace = PathBuf::from("/tmp/test");
    let source_env = HashMap::new();
    let ro_dirs = resolve_ro_bindmounts();
    let ro_etc = resolve_ro_etc_files();
    let config = NsjailConfig {
        options,
        ro_dirs: &ro_dirs,
        ro_etc_files: &ro_etc,
    };
    let cmd = build_nsjail_command(&workspace, "echo hi", &source_env, &config);
    cmd.as_std()
        .get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

#[tokio::test]
async fn test_exec_echo() {
    let (tool, _tmp) = test_exec(false);
    let result = tool.execute(r#"{"command": "echo hello"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("hello"));
}

#[tokio::test]
async fn test_exec_dangerous_command_blocked() {
    let (tool, _tmp) = test_exec(false);
    let result = tool.execute(r#"{"command": "rm -rf /"}"#).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_exec_missing_command_arg() {
    let (tool, _tmp) = test_exec(false);
    let result = tool.execute(r#"{}"#).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_exec_timeout_kills_long_command() {
    let tmp = TempDir::new().unwrap();
    let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), false);
    let tool = ExecTool::with_timeout(
        Arc::new(tmp.path().to_path_buf()),
        Arc::new(sandbox),
        Duration::from_secs(1),
    );

    let result = tool.execute(r#"{"command": "sleep 60"}"#).await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("timed out"));
}

#[tokio::test]
async fn test_exec_strips_quecto_env_vars() {
    let tmp = TempDir::new().unwrap();
    let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), false);
    let tool = ExecTool::new(Arc::new(tmp.path().to_path_buf()), Arc::new(sandbox));

    let mut env_vars = HashMap::new();
    env_vars.insert(
        "QUECTO_PROVIDERS_OPENAI_API_KEY".to_string(),
        "sk-secret".to_string(),
    );
    env_vars.insert("HOME".to_string(), "/home/testuser".to_string());

    let result = tool
        .execute_with_env(
            r#"{"command": "printenv QUECTO_PROVIDERS_OPENAI_API_KEY"}"#,
            &env_vars,
        )
        .await
        .unwrap();
    assert!(!result.content.contains("sk-secret"));
}

#[test]
fn test_nsjail_falls_back_when_binary_missing() {
    let tmp = TempDir::new().unwrap();
    let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), false);
    let opts = ExecOptions {
        isolation_mode: ExecIsolationMode::Nsjail,
        allow_native_fallback: true,
        nsjail: NsjailOptions {
            binary: "definitely-not-a-real-binary".to_string(),
            ..NsjailOptions::default()
        },
        ..ExecOptions::default()
    };
    let tool = ExecTool::with_options(Arc::new(tmp.path().to_path_buf()), Arc::new(sandbox), opts);
    assert_eq!(tool.mode(), ExecIsolationMode::Native);
    assert!(
        tool.startup_warning()
            .unwrap_or_default()
            .contains("falling back to native")
    );
    assert!(tool.startup_error().is_none());
}

#[tokio::test]
async fn test_nsjail_missing_without_fallback_returns_config_error() {
    let tmp = TempDir::new().unwrap();
    let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), false);
    let opts = ExecOptions {
        isolation_mode: ExecIsolationMode::Nsjail,
        nsjail: NsjailOptions {
            binary: "definitely-not-a-real-binary".to_string(),
            ..NsjailOptions::default()
        },
        ..ExecOptions::default()
    };
    let tool = ExecTool::with_options(Arc::new(tmp.path().to_path_buf()), Arc::new(sandbox), opts);

    let result = tool.execute(r#"{"command":"echo hi"}"#).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("allow_native_fallback")
    );
}

// --- rlimit argument tests ---

#[test]
fn test_nsjail_command_includes_rlimit_as_for_memory() {
    let args = nsjail_args_str(&NsjailOptions {
        memory_limit_mb: Some(256),
        ..NsjailOptions::default()
    });
    assert!(args.contains("--rlimit_as"), "missing --rlimit_as: {args}");
    assert!(args.contains("256"), "missing value 256: {args}");
}

#[test]
fn test_nsjail_command_includes_rlimit_nproc_for_pid_limit() {
    let args = nsjail_args_str(&NsjailOptions {
        pid_limit: Some(64),
        ..NsjailOptions::default()
    });
    assert!(
        args.contains("--rlimit_nproc"),
        "missing --rlimit_nproc: {args}"
    );
    assert!(args.contains("64"), "missing value 64: {args}");
}

#[test]
fn test_nsjail_command_includes_rlimit_cpu() {
    let args = nsjail_args_str(&NsjailOptions {
        cpu_time_limit_secs: Some(15),
        ..NsjailOptions::default()
    });
    assert!(
        args.contains("--rlimit_cpu"),
        "missing --rlimit_cpu: {args}"
    );
    assert!(args.contains("15"), "missing value 15: {args}");
}

#[test]
fn test_nsjail_command_includes_disable_clone_newcgroup() {
    let args = nsjail_args_str(&NsjailOptions::default());
    assert!(
        args.contains("--disable_clone_newcgroup"),
        "missing --disable_clone_newcgroup: {args}"
    );
}

#[test]
fn test_nsjail_command_does_not_include_cgroup_args() {
    let args = nsjail_args_str(&NsjailOptions {
        memory_limit_mb: Some(512),
        pid_limit: Some(256),
        ..NsjailOptions::default()
    });
    assert!(
        !args.contains("--cgroup_mem_max"),
        "must NOT include --cgroup_mem_max: {args}"
    );
    assert!(
        !args.contains("--cgroup_pids_max"),
        "must NOT include --cgroup_pids_max: {args}"
    );
    assert!(
        !args.contains("--detect_cgroupv2"),
        "must NOT include --detect_cgroupv2: {args}"
    );
}

#[test]
fn test_nsjail_command_includes_system_ro_bindmounts() {
    let args = nsjail_args_str(&NsjailOptions::default());
    assert!(
        args.contains("--bindmount_ro"),
        "missing --bindmount_ro: {args}"
    );
    assert!(args.contains("/usr:/usr"), "missing /usr mount: {args}");
}

#[test]
fn test_nsjail_command_does_not_mount_etc_directory() {
    let args = nsjail_args_str(&NsjailOptions::default());
    // Should mount individual /etc files, not the whole /etc directory.
    assert!(
        !args.contains("/etc:/etc"),
        "must NOT mount /etc as a whole directory: {args}"
    );
}

#[test]
fn test_nsjail_command_omits_none_limits() {
    let args = nsjail_args_str(&NsjailOptions {
        memory_limit_mb: None,
        pid_limit: None,
        cpu_time_limit_secs: None,
        wall_time_limit_secs: None,
        ..NsjailOptions::default()
    });
    assert!(
        !args.contains("--rlimit_as"),
        "unexpected --rlimit_as: {args}"
    );
    assert!(
        !args.contains("--rlimit_nproc"),
        "unexpected --rlimit_nproc: {args}"
    );
    assert!(
        !args.contains("--rlimit_cpu"),
        "unexpected --rlimit_cpu: {args}"
    );
    assert!(
        !args.contains("--time_limit"),
        "unexpected --time_limit: {args}"
    );
}

#[test]
fn test_nsjail_command_includes_time_limit() {
    let args = nsjail_args_str(&NsjailOptions {
        wall_time_limit_secs: Some(20),
        ..NsjailOptions::default()
    });
    assert!(
        args.contains("--time_limit"),
        "missing --time_limit: {args}"
    );
    assert!(args.contains("20"), "missing value 20: {args}");
}
