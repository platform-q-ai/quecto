use super::*;
use tempfile::TempDir;

fn test_exec(restrict: bool) -> (ExecTool, TempDir) {
    let tmp = TempDir::new().unwrap();
    let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), restrict);
    let tool = ExecTool::new(Arc::new(tmp.path().to_path_buf()), Arc::new(sandbox));
    (tool, tmp)
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
    let workspace = PathBuf::from("/tmp/test");
    let source_env = HashMap::new();
    let options = NsjailOptions {
        memory_limit_mb: Some(256),
        ..NsjailOptions::default()
    };
    let cmd = build_nsjail_command(&workspace, "echo hi", &source_env, &options);
    let args: Vec<String> = cmd
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect();
    let args_str = args.join(" ");
    assert!(
        args_str.contains("--rlimit_as"),
        "nsjail command must include --rlimit_as, got: {args_str}"
    );
    assert!(
        args_str.contains("256"),
        "rlimit_as must be set to 256, got: {args_str}"
    );
}

#[test]
fn test_nsjail_command_includes_rlimit_nproc_for_pid_limit() {
    let workspace = PathBuf::from("/tmp/test");
    let source_env = HashMap::new();
    let options = NsjailOptions {
        pid_limit: Some(64),
        ..NsjailOptions::default()
    };
    let cmd = build_nsjail_command(&workspace, "echo hi", &source_env, &options);
    let args: Vec<String> = cmd
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect();
    let args_str = args.join(" ");
    assert!(
        args_str.contains("--rlimit_nproc"),
        "nsjail command must include --rlimit_nproc, got: {args_str}"
    );
    assert!(
        args_str.contains("64"),
        "rlimit_nproc must be set to 64, got: {args_str}"
    );
}

#[test]
fn test_nsjail_command_includes_rlimit_cpu() {
    let workspace = PathBuf::from("/tmp/test");
    let source_env = HashMap::new();
    let options = NsjailOptions {
        cpu_time_limit_secs: Some(15),
        ..NsjailOptions::default()
    };
    let cmd = build_nsjail_command(&workspace, "echo hi", &source_env, &options);
    let args: Vec<String> = cmd
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect();
    let args_str = args.join(" ");
    assert!(
        args_str.contains("--rlimit_cpu"),
        "nsjail command must include --rlimit_cpu, got: {args_str}"
    );
    assert!(
        args_str.contains("15"),
        "rlimit_cpu must be set to 15, got: {args_str}"
    );
}

#[test]
fn test_nsjail_command_includes_disable_clone_newcgroup() {
    let workspace = PathBuf::from("/tmp/test");
    let source_env = HashMap::new();
    let options = NsjailOptions::default();
    let cmd = build_nsjail_command(&workspace, "echo hi", &source_env, &options);
    let args: Vec<String> = cmd
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect();
    let args_str = args.join(" ");
    assert!(
        args_str.contains("--disable_clone_newcgroup"),
        "nsjail command must include --disable_clone_newcgroup, got: {args_str}"
    );
}

#[test]
fn test_nsjail_command_does_not_include_cgroup_args() {
    let workspace = PathBuf::from("/tmp/test");
    let source_env = HashMap::new();
    let options = NsjailOptions {
        memory_limit_mb: Some(512),
        pid_limit: Some(256),
        ..NsjailOptions::default()
    };
    let cmd = build_nsjail_command(&workspace, "echo hi", &source_env, &options);
    let args: Vec<String> = cmd
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect();
    let args_str = args.join(" ");
    assert!(
        !args_str.contains("--cgroup_mem_max"),
        "nsjail command must NOT include --cgroup_mem_max, got: {args_str}"
    );
    assert!(
        !args_str.contains("--cgroup_pids_max"),
        "nsjail command must NOT include --cgroup_pids_max, got: {args_str}"
    );
    assert!(
        !args_str.contains("--detect_cgroupv2"),
        "nsjail command must NOT include --detect_cgroupv2, got: {args_str}"
    );
}

#[test]
fn test_nsjail_command_includes_system_ro_bindmounts() {
    let workspace = PathBuf::from("/tmp/test");
    let source_env = HashMap::new();
    let options = NsjailOptions::default();
    let cmd = build_nsjail_command(&workspace, "echo hi", &source_env, &options);
    let args: Vec<String> = cmd
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect();
    let args_str = args.join(" ");
    assert!(
        args_str.contains("--bindmount_ro"),
        "nsjail command must include --bindmount_ro for system paths, got: {args_str}"
    );
    // /usr should be mounted read-only
    assert!(
        args_str.contains("/usr:/usr"),
        "must mount /usr read-only, got: {args_str}"
    );
}

#[test]
fn test_nsjail_command_omits_none_limits() {
    let workspace = PathBuf::from("/tmp/test");
    let source_env = HashMap::new();
    let options = NsjailOptions {
        memory_limit_mb: None,
        pid_limit: None,
        cpu_time_limit_secs: None,
        wall_time_limit_secs: None,
        ..NsjailOptions::default()
    };
    let cmd = build_nsjail_command(&workspace, "echo hi", &source_env, &options);
    let args: Vec<String> = cmd
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect();
    let args_str = args.join(" ");
    assert!(
        !args_str.contains("--rlimit_as"),
        "should not include --rlimit_as when None, got: {args_str}"
    );
    assert!(
        !args_str.contains("--rlimit_nproc"),
        "should not include --rlimit_nproc when None, got: {args_str}"
    );
    assert!(
        !args_str.contains("--rlimit_cpu"),
        "should not include --rlimit_cpu when None, got: {args_str}"
    );
    assert!(
        !args_str.contains("--time_limit"),
        "should not include --time_limit when None, got: {args_str}"
    );
}

#[test]
fn test_nsjail_command_includes_time_limit() {
    let workspace = PathBuf::from("/tmp/test");
    let source_env = HashMap::new();
    let options = NsjailOptions {
        wall_time_limit_secs: Some(20),
        ..NsjailOptions::default()
    };
    let cmd = build_nsjail_command(&workspace, "echo hi", &source_env, &options);
    let args: Vec<String> = cmd
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect();
    let args_str = args.join(" ");
    assert!(
        args_str.contains("--time_limit"),
        "nsjail command must include --time_limit, got: {args_str}"
    );
    assert!(
        args_str.contains("20"),
        "time_limit must be set to 20, got: {args_str}"
    );
}
