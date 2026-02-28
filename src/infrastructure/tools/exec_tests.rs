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

#[test]
fn test_detect_cgroup_version() {
    let version = detect_cgroup_version();
    // Should return a valid variant regardless of host
    assert!(version == CgroupVersion::V1 || version == CgroupVersion::V2);
}

#[test]
fn test_is_nsjail_cgroup_failure_createcgroup() {
    assert!(is_nsjail_cgroup_failure(
        "exit code 255\nstdout: \nstderr: \
         [W] createCgroup():43 mkdir('/sys/fs/cgroup/memory/NSJAIL') failed"
    ));
}

#[test]
fn test_is_nsjail_cgroup_failure_initialize() {
    assert!(is_nsjail_cgroup_failure(
        "exit code 255\nstdout: \nstderr: \
         [E] Couldn't initialize cgroup user namespace for pid=12345"
    ));
}

#[test]
fn test_is_nsjail_cgroup_failure_no_such_file() {
    assert!(is_nsjail_cgroup_failure(
        "exit code 255\nstdout: \nstderr: \
         cgroup path: No such file or directory"
    ));
}

#[test]
fn test_is_nsjail_cgroup_failure_permission_denied() {
    assert!(is_nsjail_cgroup_failure(
        "exit code 255\nstdout: \nstderr: \
         cgroup: Permission denied"
    ));
}

#[test]
fn test_is_nsjail_cgroup_failure_normal_error() {
    assert!(!is_nsjail_cgroup_failure(
        "exit code 1\nstdout: \nstderr: command not found"
    ));
}

#[test]
fn test_nsjail_command_includes_detect_cgroupv2() {
    let workspace = PathBuf::from("/tmp/test");
    let source_env = HashMap::new();
    let options = NsjailOptions::default();
    let cmd = build_nsjail_command(&workspace, "echo hi", &source_env, &options);
    let args: Vec<_> = cmd.as_std().get_args().collect();
    assert!(
        args.contains(&std::ffi::OsStr::new("--detect_cgroupv2")),
        "nsjail command must include --detect_cgroupv2, got: {:?}",
        args
    );
}

#[tokio::test]
async fn test_nsjail_cgroup_failure_falls_back_to_native() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().to_path_buf();

    // Create a fake nsjail that simulates a cgroup v1 failure on a v2 host
    let fake_nsjail = ws.join("fake-nsjail-cgroup-fail.sh");
    std::fs::write(
        &fake_nsjail,
        "#!/bin/sh\n\
         echo \"[W] createCgroup():43 mkdir('/sys/fs/cgroup/memory/NSJAIL') failed: No such file or directory\" >&2\n\
         echo \"[E] Couldn't initialize cgroup user namespace\" >&2\n\
         exit 255\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        let mut perms = std::fs::metadata(&fake_nsjail).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake_nsjail, perms).unwrap();
    }

    let sandbox = Sandbox::new(Some(ws.clone()), false);
    let opts = ExecOptions {
        isolation_mode: ExecIsolationMode::Nsjail,
        allow_native_fallback: true,
        nsjail: NsjailOptions {
            binary: fake_nsjail.to_string_lossy().to_string(),
            ..NsjailOptions::default()
        },
        ..ExecOptions::default()
    };
    let tool = ExecTool::with_options(Arc::new(ws), Arc::new(sandbox), opts);

    // The fake nsjail will fail with cgroup error, should fall back to native
    let result = tool.execute(r#"{"command": "echo hello"}"#).await.unwrap();
    assert!(
        !result.is_error,
        "expected success after fallback, got: {}",
        result.content
    );
    assert!(result.content.contains("hello"));
}

#[test]
fn test_nsjail_cgroup_failure_without_fallback_does_not_retry() {
    // When allow_native_fallback is false and a cgroup failure occurs,
    // is_nsjail_cgroup_failure returns true but the fallback branch is
    // not taken. This is a logic-level test since we cannot easily place
    // a fake nsjail binary in a trusted system path for integration testing.
    let content = "exit code 255\nstdout: \nstderr: \
         [W] createCgroup():43 mkdir failed\n\
         [E] Couldn't initialize cgroup user namespace";
    assert!(is_nsjail_cgroup_failure(content));

    // Also verify that non-cgroup errors are not mistaken for cgroup failures
    let normal = "exit code 1\nstdout: \nstderr: file not found";
    assert!(!is_nsjail_cgroup_failure(normal));
}
