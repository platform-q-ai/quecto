use super::*;

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

// --- Default resource limit tests ---

#[test]
fn test_nsjail_default_memory_limit_is_4096_mb() {
    let args = nsjail_args_str(&NsjailOptions::default());
    assert!(args.contains("--rlimit_as"), "missing --rlimit_as: {args}");
    assert!(
        args.contains("4096"),
        "default rlimit_as should be 4096 MB: {args}"
    );
}

#[test]
fn test_nsjail_default_cpu_time_limit_is_28800_secs() {
    // 2 cores × 14 400 s wall budget = 28 800 CPU-seconds.
    let args = nsjail_args_str(&NsjailOptions::default());
    assert!(
        args.contains("--rlimit_cpu"),
        "missing --rlimit_cpu: {args}"
    );
    assert!(
        args.contains("28800"),
        "default rlimit_cpu should be 28800 CPU-s: {args}"
    );
}

#[test]
fn test_nsjail_default_wall_time_limit_is_14400_secs() {
    // 4 hours = 14 400 wall-clock seconds.
    let args = nsjail_args_str(&NsjailOptions::default());
    assert!(
        args.contains("--time_limit"),
        "missing --time_limit: {args}"
    );
    assert!(
        args.contains("14400"),
        "default --time_limit should be 14400 s: {args}"
    );
}

#[test]
fn test_nsjail_default_tmp_size_is_512_mb() {
    // 512 MB = 536_870_912 bytes; conservative for RPi/VPS targets.
    let args = nsjail_args_str(&NsjailOptions::default());
    assert!(
        args.contains("none:/tmp:tmpfs:size="),
        "missing tmpfs mount: {args}"
    );
    assert!(
        args.contains("536870912"),
        "default tmp should be 512 MB: {args}"
    );
}

#[test]
fn test_nsjail_tmp_size_is_configurable() {
    let opts = NsjailOptions {
        tmp_size_mb: Some(1024),
        ..NsjailOptions::default()
    };
    let args = nsjail_args_str(&opts);
    let expected = (1024u64 * 1024 * 1024).to_string();
    assert!(
        args.contains(&expected),
        "expected 1 GiB ({expected}) in args, got: {args}"
    );
}

#[test]
fn test_exec_options_default_timeout_matches_nsjail_wall_limit() {
    // Tokio timeout must be >= --time_limit so nsjail fires first.
    use crate::infrastructure::tools::bash::DEFAULT_NSJAIL_WALL_TIME_LIMIT_SECS;
    use std::time::Duration;
    let exec_default = ExecOptions::default();
    let nsjail_wall = Duration::from_secs(DEFAULT_NSJAIL_WALL_TIME_LIMIT_SECS);
    assert!(
        exec_default.timeout >= nsjail_wall,
        "ExecOptions default timeout ({:?}) must be >= nsjail wall limit ({:?})",
        exec_default.timeout,
        nsjail_wall
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
