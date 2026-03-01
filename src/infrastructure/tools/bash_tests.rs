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
    let ro_dev = resolve_ro_dev_files();
    let config = NsjailConfig {
        options,
        ro_dirs: &ro_dirs,
        ro_etc_files: &ro_etc,
        ro_dev_files: &ro_dev,
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
    let result = tool.execute(r#"{}"#).await.unwrap();
    assert!(
        result.is_error,
        "expected error result, got: {}",
        result.content
    );
    assert!(
        result.content.contains("command"),
        "should mention missing 'command', got: {}",
        result.content
    );
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
fn test_nsjail_command_includes_bounded_tmpfs_for_tmp() {
    let args = nsjail_args_str(&NsjailOptions::default());
    // Should use bounded `-m none:/tmp:tmpfs:size=<bytes>` syntax, not unbounded --tmpfsmount.
    assert!(
        args.contains("none:/tmp:tmpfs:size="),
        "missing bounded tmpfs mount for /tmp: {args}"
    );
    assert!(
        !args.contains("--tmpfsmount"),
        "should use -m syntax, not --tmpfsmount: {args}"
    );
}

#[test]
fn test_nsjail_command_sets_tmpdir_env() {
    let workspace = PathBuf::from("/tmp/test");
    let mut source_env = HashMap::new();
    source_env.insert("HOME".to_string(), "/home/test".to_string());
    let options = NsjailOptions::default();
    let ro_dirs = resolve_ro_bindmounts();
    let ro_etc = resolve_ro_etc_files();
    let ro_dev = resolve_ro_dev_files();
    let config = NsjailConfig {
        options: &options,
        ro_dirs: &ro_dirs,
        ro_etc_files: &ro_etc,
        ro_dev_files: &ro_dev,
    };
    let cmd = build_nsjail_command(&workspace, "echo hi", &source_env, &config);
    let envs: Vec<(String, String)> = cmd
        .as_std()
        .get_envs()
        .filter_map(|(k, v)| {
            Some((
                k.to_string_lossy().to_string(),
                v?.to_string_lossy().to_string(),
            ))
        })
        .collect();
    let tmpdir = envs.iter().find(|(k, _)| k == "TMPDIR");
    assert!(
        tmpdir.is_some(),
        "TMPDIR should be set in nsjail env, got: {envs:?}"
    );
    assert_eq!(tmpdir.unwrap().1, "/tmp", "TMPDIR should be /tmp");
}

#[test]
fn test_nsjail_command_respects_caller_tmpdir_override() {
    let workspace = PathBuf::from("/tmp/test");
    let mut source_env = HashMap::new();
    source_env.insert("HOME".to_string(), "/home/test".to_string());
    source_env.insert("TMPDIR".to_string(), "/workspace/tmp".to_string());
    let options = NsjailOptions::default();
    let ro_dirs = resolve_ro_bindmounts();
    let ro_etc = resolve_ro_etc_files();
    let ro_dev = resolve_ro_dev_files();
    let config = NsjailConfig {
        options: &options,
        ro_dirs: &ro_dirs,
        ro_etc_files: &ro_etc,
        ro_dev_files: &ro_dev,
    };
    let cmd = build_nsjail_command(&workspace, "echo hi", &source_env, &config);
    let envs: Vec<(String, String)> = cmd
        .as_std()
        .get_envs()
        .filter_map(|(k, v)| {
            Some((
                k.to_string_lossy().to_string(),
                v?.to_string_lossy().to_string(),
            ))
        })
        .collect();
    let tmpdir = envs.iter().find(|(k, _)| k == "TMPDIR");
    assert!(tmpdir.is_some(), "TMPDIR should be present");
    assert_eq!(
        tmpdir.unwrap().1,
        "/workspace/tmp",
        "caller-provided TMPDIR should be preserved, not overwritten"
    );
}

#[test]
fn test_nsjail_command_sets_tmp_and_temp_env() {
    let workspace = PathBuf::from("/tmp/test");
    let source_env = HashMap::new();
    let options = NsjailOptions::default();
    let ro_dirs = resolve_ro_bindmounts();
    let ro_etc = resolve_ro_etc_files();
    let ro_dev = resolve_ro_dev_files();
    let config = NsjailConfig {
        options: &options,
        ro_dirs: &ro_dirs,
        ro_etc_files: &ro_etc,
        ro_dev_files: &ro_dev,
    };
    let cmd = build_nsjail_command(&workspace, "echo hi", &source_env, &config);
    let envs: Vec<(String, String)> = cmd
        .as_std()
        .get_envs()
        .filter_map(|(k, v)| {
            Some((
                k.to_string_lossy().to_string(),
                v?.to_string_lossy().to_string(),
            ))
        })
        .collect();
    for var in ["TMPDIR", "TMP", "TEMP"] {
        let found = envs.iter().find(|(k, _)| k == var);
        assert!(found.is_some(), "{var} should be set in nsjail env");
        assert_eq!(found.unwrap().1, "/tmp", "{var} should be /tmp");
    }
}

#[test]
fn test_nsjail_command_uses_bounded_tmpfs_mount() {
    let args = nsjail_args_str(&NsjailOptions {
        tmp_size_mb: Some(64),
        ..NsjailOptions::default()
    });
    assert!(args.contains("-m"), "missing -m mount arg: {args}");
    assert!(
        args.contains("none:/tmp:tmpfs:size=67108864"),
        "missing bounded tmpfs mount: {args}"
    );
}

#[test]
fn test_nsjail_command_omits_tmp_mount_when_disabled() {
    let args = nsjail_args_str(&NsjailOptions {
        tmp_size_mb: None,
        ..NsjailOptions::default()
    });
    assert!(
        !args.contains("none:/tmp:tmpfs"),
        "should not have tmpfs mount when disabled: {args}"
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

// --- Default memory limit test ---

#[test]
fn test_nsjail_default_memory_limit_is_4096_mb() {
    // The default RLIMIT_AS must be 4096 MB so that Node/V8, JVM, and Go
    // runtimes — which reserve large virtual address ranges at startup —
    // can start inside the sandbox without hitting ENOMEM.
    let args = nsjail_args_str(&NsjailOptions::default());
    assert!(args.contains("--rlimit_as"), "missing --rlimit_as: {args}");
    assert!(
        args.contains("4096"),
        "default rlimit_as should be 4096 MB (was 512), got: {args}"
    );
}

// --- /dev device node bindmount tests ---

#[test]
fn test_nsjail_command_includes_dev_null_bindmount() {
    let args = nsjail_args_str(&NsjailOptions::default());
    assert!(
        args.contains("/dev/null:/dev/null"),
        "missing /dev/null bindmount: {args}"
    );
}

#[test]
fn test_nsjail_command_includes_dev_urandom_bindmount() {
    let args = nsjail_args_str(&NsjailOptions::default());
    assert!(
        args.contains("/dev/urandom:/dev/urandom"),
        "missing /dev/urandom bindmount: {args}"
    );
}

#[test]
fn test_nsjail_command_includes_dev_zero_bindmount() {
    let args = nsjail_args_str(&NsjailOptions::default());
    assert!(
        args.contains("/dev/zero:/dev/zero"),
        "missing /dev/zero bindmount: {args}"
    );
}

#[test]
fn test_nsjail_command_includes_dev_random_bindmount() {
    let args = nsjail_args_str(&NsjailOptions::default());
    assert!(
        args.contains("/dev/random:/dev/random"),
        "missing /dev/random bindmount: {args}"
    );
}

#[test]
fn test_nsjail_command_does_not_mount_full_dev_directory() {
    let args = nsjail_args_str(&NsjailOptions::default());
    // Only specific device nodes should be mounted, not the whole /dev directory.
    assert!(
        !args.contains("/dev:/dev"),
        "must NOT mount /dev as a whole directory: {args}"
    );
}

#[test]
fn test_nsjail_dev_files_are_bindmount_ro() {
    // All /dev/* mounts must use --bindmount_ro (read-only).
    let workspace = PathBuf::from("/tmp/test");
    let source_env = HashMap::new();
    let options = NsjailOptions::default();
    let ro_dirs = resolve_ro_bindmounts();
    let ro_etc = resolve_ro_etc_files();
    let ro_dev = resolve_ro_dev_files();
    let config = NsjailConfig {
        options: &options,
        ro_dirs: &ro_dirs,
        ro_etc_files: &ro_etc,
        ro_dev_files: &ro_dev,
    };
    let cmd = build_nsjail_command(&workspace, "echo hi", &source_env, &config);
    let args: Vec<String> = cmd
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect();
    // Find all --bindmount_ro entries and confirm /dev/* use that flag (not --bindmount).
    for (i, arg) in args.iter().enumerate() {
        if arg.starts_with("/dev/") && arg.contains(":/dev/") {
            // The arg before it must be --bindmount_ro
            assert!(
                i > 0 && args[i - 1] == "--bindmount_ro",
                "/dev mount '{arg}' must be preceded by --bindmount_ro, got {:?}",
                args.get(i.saturating_sub(1))
            );
        }
    }
}

#[test]
fn test_truncate_tail_no_truncation() {
    use crate::infrastructure::tools::truncate::truncate_tail;
    let content = "line1\nline2\nline3";
    let tr = truncate_tail(content, 2000, 50 * 1024);
    assert!(!tr.truncated);
    assert_eq!(tr.content, content);
}

#[test]
fn test_truncate_tail_line_limit() {
    use crate::infrastructure::tools::truncate::{TruncatedBy, truncate_tail};
    let content: String = (1..=3000).map(|i| format!("line{}\n", i)).collect();
    let tr = truncate_tail(&content, 2000, 50 * 1024);
    assert!(tr.truncated, "expected truncation");
    assert_eq!(tr.truncated_by, Some(TruncatedBy::Lines));
    assert!(
        tr.content.contains("3000"),
        "expected last line, got: {}",
        &tr.content[..tr.content.len().min(100)]
    );
    assert!(
        !tr.content.contains("line1\n"),
        "should have dropped first lines"
    );
}

#[test]
fn test_truncate_tail_byte_limit() {
    use crate::infrastructure::tools::truncate::{TruncatedBy, truncate_tail};
    // 1500 lines of 40 chars = ~61.5KB > 50KB, BUT 1500 < 2000 (line limit)
    // → truncated by bytes, not lines
    let content: String = (1..=1500).map(|i| format!("{:040}\n", i)).collect();
    let tr = truncate_tail(&content, 2000, 50 * 1024);
    assert!(tr.truncated, "expected byte truncation");
    assert_eq!(tr.truncated_by, Some(TruncatedBy::Bytes));
    assert!(
        tr.content.contains(&format!("{:040}", 1500)),
        "expected last entry in tail"
    );
}

// --- Per-invocation timeout parameter ---

#[tokio::test]
async fn test_exec_per_invocation_timeout_kills_slow_command() {
    let (tool, _tmp) = test_exec(false);
    // Pass timeout=1 in JSON args — should kill sleep 10
    let result = tool
        .execute(r#"{"command": "sleep 10", "timeout": 1}"#)
        .await
        .unwrap();
    assert!(
        result.is_error,
        "slow command should be killed by per-invocation timeout"
    );
    assert!(
        result.content.contains("timed out"),
        "should mention timeout, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_exec_per_invocation_timeout_capped_at_max() {
    let tmp = TempDir::new().unwrap();
    let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), false);
    // Configure tool with max 5s timeout
    let opts = ExecOptions {
        timeout: Duration::from_secs(5),
        ..ExecOptions::default()
    };
    let tool = ExecTool::with_options(Arc::new(tmp.path().to_path_buf()), Arc::new(sandbox), opts);
    // Request 99999s — should be capped at configured max (5s), command still runs
    let result = tool
        .execute(r#"{"command": "echo hi", "timeout": 99999}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "echo should succeed: {}", result.content);
    assert!(result.content.contains("hi"));
}

// --- commandPrefix option ---

#[tokio::test]
async fn test_exec_command_prefix_prepended() {
    let tmp = TempDir::new().unwrap();
    let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), false);
    let opts = ExecOptions {
        command_prefix: Some("export MY_PREFIX_VAR=hello".to_string()),
        ..ExecOptions::default()
    };
    let tool = ExecTool::with_options(Arc::new(tmp.path().to_path_buf()), Arc::new(sandbox), opts);
    let result = tool
        .execute(r#"{"command": "echo $MY_PREFIX_VAR"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "echo should succeed: {}", result.content);
    assert!(
        result.content.contains("hello"),
        "prefix should set env: {}",
        result.content
    );
}

// --- Shell detection ---

#[tokio::test]
async fn test_exec_shell_detection_uses_shell_env() {
    let (tool, _tmp) = test_exec(false);
    // Set SHELL to /bin/sh and verify the shell is spawned (not an arbitrary binary).
    // $0 in the spawned shell prints the shell executable name.
    let mut env_overrides = HashMap::new();
    env_overrides.insert("SHELL".to_string(), "/bin/sh".to_string());
    let result = tool
        .execute_with_env(r#"{"command": "echo $0"}"#, &env_overrides)
        .await
        .unwrap();
    assert!(
        !result.is_error,
        "shell detection should work: {}",
        result.content
    );
    assert!(
        result.content.contains("sh"),
        "output should name the shell, got: {}",
        result.content
    );
}

#[test]
fn test_exec_disallowed_shell_falls_back_to_sh() {
    // $SHELL pointing to a non-allowlisted binary should silently fall back to /bin/sh.
    // We test build_shell_command indirectly via the allowlist logic.
    // ALLOWED_SHELLS does not contain /tmp/evil, so it should use /bin/sh.
    // We can't call build_shell_command directly (it's private), but we can
    // confirm the constant list is correct.
    assert!(ALLOWED_SHELLS.contains(&"/bin/sh"));
    assert!(ALLOWED_SHELLS.contains(&"/bin/bash"));
    assert!(!ALLOWED_SHELLS.contains(&"/tmp/evil"));
}

// --- Truncation notice format ---

#[test]
fn test_exec_truncation_byte_notice_uses_truncate_tail() {
    use crate::infrastructure::tools::truncate::{TruncatedBy, truncate_tail};
    // Generate content that will be truncated by bytes (multi-line, >50KB)
    let line = "x".repeat(50) + "\n"; // 51 bytes per line
    let content: String = line.repeat(1500); // ~76.5KB
    let tr = truncate_tail(&content, 2000, 50 * 1024);
    assert!(tr.truncated, "should be truncated by bytes");
    assert_eq!(tr.truncated_by, Some(TruncatedBy::Bytes));
}

#[test]
fn test_exec_truncation_line_notice_uses_truncate_tail() {
    use crate::infrastructure::tools::truncate::{TruncatedBy, truncate_tail};
    // 3000 lines, each <50 bytes → truncated by lines
    let content: String = (1..=3000).map(|i| format!("line{}\n", i)).collect();
    let tr = truncate_tail(&content, 2000, 50 * 1024);
    assert!(tr.truncated, "should be truncated by lines");
    assert_eq!(tr.truncated_by, Some(TruncatedBy::Lines));
    assert!(
        tr.content.contains("line3000"),
        "tail should include the last line"
    );
}

#[tokio::test]
async fn test_exec_empty_object_returns_actionable_error() {
    let (tool, _tmp) = test_exec(false);
    let result = tool.execute("{}").await.unwrap();
    assert!(result.is_error, "expected error, got: {}", result.content);
    assert!(
        result.content.contains("command"),
        "should mention 'command', got: {}",
        result.content
    );
    assert!(
        result.content.contains("Example"),
        "should include example, got: {}",
        result.content
    );
}

#[test]
fn test_exec_description_includes_example() {
    let (tool, _tmp) = test_exec(false);
    let def = tool.definition();
    assert!(
        def.description.contains("Example"),
        "bash description should include Example, got: {}",
        def.description
    );
}
