use super::*;
use tempfile::TempDir;
use tokio::time::{Duration, timeout};

fn request(pattern: &str) -> FindPathsRequest {
    FindPathsRequest {
        pattern: pattern.into(),
        path: ".".into(),
        limit: 1000,
    }
}

fn fixture(script: &str) -> (TempDir, FdFindPaths) {
    let dir = TempDir::new().unwrap();
    let binary = dir.path().join("fake-fd");
    // A parallel test can fork while this process holds a writable script fd,
    // inheriting it until exec and making our exec fail with ETXTBSY. Create
    // scripts in a separate writer process and join it before execution.
    let written = std::process::Command::new("/usr/bin/python3")
        .arg("-c")
        .arg("import os,sys\np=sys.argv[1]\nwith open(p,'w') as f: f.write(sys.argv[2]); f.flush(); os.fsync(f.fileno())\nos.chmod(p,0o755)")
        .arg(&binary)
        .arg(format!("#!/usr/bin/python3\n{script}\n"))
        .status().unwrap();
    assert!(
        written.success(),
        "fixture writer must finish before execution"
    );
    let effect = FdFindPaths::with_fd_binary(
        Arc::new(dir.path().into()),
        Arc::new(Sandbox::new(None)),
        binary.to_string_lossy().into(),
    );
    (dir, effect)
}

async fn run(effect: &FdFindPaths) -> Result<FindOutput, FindError> {
    timeout(Duration::from_secs(5), effect.find(request("*")))
        .await
        .expect("fd must not deadlock")
}

#[tokio::test]
async fn natural_status_matrix() {
    for code in [0, 1, 2, 3] {
        for output in ["", "entry\\n"] {
            let (_dir, effect) = fixture(&format!(
                "import sys\nsys.stdout.write('{output}')\nsys.stderr.write('diagnostic')\nsys.exit({code})"
            ));
            let result = run(&effect).await;
            match (code, output.is_empty()) {
                (0 | 1, _) => assert!(
                    result.is_ok(),
                    "code={code} output={output:?} result={result:?}"
                ),
                (3, false) => assert!(result.unwrap().incomplete),
                _ => assert!(matches!(result, Err(FindError::Search(_)))),
            }
        }
    }
}

#[tokio::test]
async fn either_pipe_pressure_stops_and_reaps() {
    for stream in ["stdout", "stderr"] {
        let (dir, effect) = fixture(&format!(
            "import os,sys\nopen('pid','w').write(str(os.getpid()))\nwhile True: os.write(sys.{stream}.fileno(), b'x'*8192)"
        ));
        let result = run(&effect).await.unwrap();
        assert!(result.incomplete);
        assert!(result.entries.is_empty());
        assert_reaped(&dir).await;
    }
}

async fn ready_pid(dir: &TempDir) -> u32 {
    timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(text) = tokio::fs::read_to_string(dir.path().join("pid")).await {
                if let Ok(pid) = text.parse() {
                    return pid;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("fixture readiness")
}

async fn assert_reaped(dir: &TempDir) {
    let pid = ready_pid(dir).await;
    timeout(Duration::from_secs(5), async {
        loop {
            if std::path::Path::new(&format!("/proc/{pid}")).exists() {
                tokio::task::yield_now().await;
            } else {
                break;
            }
        }
    })
    .await
    .expect("child must terminate AND disappear, including zombie");
}

#[tokio::test]
async fn dropped_call_reaps_during_streams_and_wait() {
    for preparation in ["", "os.close(1)", "os.close(1); os.close(2)"] {
        let (dir, effect) = fixture(&format!(
            "import os,time\n{preparation}\nopen('pid','w').write(str(os.getpid()))\ntime.sleep(60)"
        ));
        let effect = Arc::new(effect);
        let mut task = tokio::spawn(async move { effect.find(request("*")).await });
        ready_or_result(&dir, &mut task).await;
        task.abort();
        let _ = task.await;
        assert_reaped(&dir).await;
    }
}

#[test]
fn normalization_preserves_components_and_complete_lines() {
    let output = normalize_output(
        b"/ws/file\n/ws2/other\n/ws/dir/\npartial",
        std::path::Path::new("/ws/"),
        true,
    );
    assert_eq!(output, ["file", "/ws2/other", "dir/"]);
    assert_eq!(
        normalize_output(b"/file\n", std::path::Path::new("/"), false),
        ["file"]
    );
    assert_eq!(
        normalize_output(b"\xff\nlast", std::path::Path::new("/ws"), false),
        ["\u{fffd}", "last"]
    );
}

#[tokio::test]
async fn argv_cwd_and_glob_are_literal() {
    let (dir, effect) = fixture(
        "import os,sys,json\nopen('argv','w').write(json.dumps(sys.argv[1:]))\nopen('cwd','w').write(os.getcwd())",
    );
    let mut req = request("./src/*.rs");
    req.limit = 7;
    effect.find(req).await.unwrap();
    let args: Vec<String> =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("argv")).unwrap()).unwrap();
    assert_eq!(
        &args[..9],
        [
            "--glob",
            "--color=never",
            "--hidden",
            "--no-require-git",
            "--max-results",
            "7",
            "--full-path",
            "--",
            "**/src/*.rs"
        ]
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("cwd")).unwrap(),
        dir.path().to_string_lossy()
    );
}

#[tokio::test]
async fn real_fd_patterns_and_directories() {
    assert!(
        std::process::Command::new("fd")
            .arg("--version")
            .status()
            .unwrap()
            .success(),
        "real fd is required"
    );
    let dir = TempDir::new().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    for name in [
        "src/main.rs",
        ".hidden.rs",
        "-dash.rs",
        "space name.rs",
        "日本.rs",
    ] {
        std::fs::write(dir.path().join(name), "").unwrap();
    }
    let effect = FdFindPaths::new(Arc::new(dir.path().into()), Arc::new(Sandbox::new(None)));
    for pattern in [
        "*.rs",
        "**/*.rs",
        "src/*.rs",
        "./src/*.rs",
        "/src/*.rs",
        "-dash.rs",
        "space name.rs",
        "日本.rs",
        "src",
    ] {
        let output = effect.find(request(pattern)).await.unwrap();
        assert!(!output.entries.is_empty(), "pattern {pattern}");
        assert!(!output.incomplete);
    }
    let output = effect.find(request("src")).await.unwrap();
    assert_eq!(output.entries, ["src/"]);
}

#[tokio::test]
async fn read_fault_is_not_eof() {
    struct Broken;
    impl tokio::io::AsyncRead for Broken {
        fn poll_read(
            self: Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            _: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Err(std::io::Error::other("injected read failure")))
        }
    }
    let mut bytes = Vec::new();
    assert!(drain(&mut Broken, 5, &mut bytes).await.is_err());
}

#[tokio::test]
async fn exact_and_over_caps_never_fabricate_partial_entry() {
    for count in [STDOUT_CAP - 1, STDOUT_CAP, STDOUT_CAP + 1] {
        let (_dir, effect) = fixture(&format!("import os\nos.write(1,b'x'*{count})"));
        let output = run(&effect).await.unwrap();
        if count < STDOUT_CAP {
            assert_eq!(output.entries[0].len(), count);
            assert!(!output.incomplete);
        } else {
            assert!(output.entries.is_empty());
            assert!(output.incomplete);
        }
    }
}

#[tokio::test]
async fn signal_status_with_partial_output_is_incomplete() {
    for data in ["", "entry\\n"] {
        let (_dir, effect) = fixture(&format!(
            "import os,signal\nos.write(1,b'{data}')\nos.kill(os.getpid(),signal.SIGTERM)"
        ));
        let result = run(&effect).await;
        if data.is_empty() {
            assert!(matches!(result, Err(FindError::Search(_))));
        } else {
            assert!(result.unwrap().incomplete);
        }
    }
}

#[tokio::test]
async fn concurrent_cancel_does_not_affect_other_call() {
    let (cancel_dir, cancel) =
        fixture("import os,time\nopen('pid','w').write(str(os.getpid()))\ntime.sleep(60)");
    let (_success_dir, success) = fixture("print('other-entry')");
    let mut task = tokio::spawn(async move { cancel.find(request("*")).await });
    ready_or_result(&cancel_dir, &mut task).await;
    task.abort();
    let _ = task.await;
    assert_eq!(run(&success).await.unwrap().entries, ["other-entry"]);
    assert_reaped(&cancel_dir).await;
}

#[tokio::test]
async fn missing_binary_errors_and_fd_status_one_preserves_legacy_delivery() {
    let dir = TempDir::new().unwrap();
    let effect = FdFindPaths::with_fd_binary(
        Arc::new(dir.path().into()),
        Arc::new(Sandbox::new(None)),
        dir.path().join("missing").to_string_lossy().into(),
    );
    assert!(
        matches!(run(&effect).await, Err(FindError::Spawn(message)) if message.contains("install fd-find"))
    );
    let effect = FdFindPaths::new(Arc::new(dir.path().into()), Arc::new(Sandbox::new(None)));
    std::fs::write(dir.path().join("file"), "").unwrap();
    for path in ["missing", "file"] {
        let mut req = request("*");
        req.path = path.into();
        let output = effect.find(req).await.unwrap();
        assert!(output.entries.is_empty());
        assert!(!output.incomplete);
    }
    // fd 10.2 emits diagnostics with status 1 for invalid roots/globs.
    // Deliberately preserve the reviewed legacy status-1 delivery policy.
    let output = effect.find(request("[")).await.unwrap();
    assert!(output.entries.is_empty());
    assert!(!output.incomplete);
}

#[tokio::test]
async fn scoped_ignores_apply_inside_and_outside_git() {
    for git in [false, true] {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("nested")).unwrap();
        if git {
            std::fs::create_dir(dir.path().join(".git")).unwrap();
        }
        for name in ["root.txt", "nested/keep.txt", "nested/drop.txt"] {
            std::fs::write(dir.path().join(name), "").unwrap();
        }
        std::fs::write(dir.path().join("nested/.gitignore"), "*.txt\n!keep.txt\n").unwrap();
        let effect = FdFindPaths::new(Arc::new(dir.path().into()), Arc::new(Sandbox::new(None)));
        let mut paths = effect.find(request("*.txt")).await.unwrap().entries;
        paths.sort();
        assert_eq!(paths, ["nested/keep.txt", "root.txt"]);
    }
}

#[tokio::test]
async fn unpolled_future_never_spawns() {
    let (dir, effect) = fixture("open('spawned','w').write('yes')");
    drop(effect.find(request("*")));
    assert!(!dir.path().join("spawned").exists());
}

#[tokio::test]
async fn both_pipes_and_closed_stdout_pressure_are_bounded() {
    for script in [
        "import os,threading\nopen('pid','w').write(str(os.getpid()))\ndef stderr():\n while True: os.write(2,b'e'*8192)\nthreading.Thread(target=stderr).start()\nwhile True: os.write(1,b'o'*8192)",
        "import os\nopen('pid','w').write(str(os.getpid()))\nos.close(1)\nwhile True: os.write(2,b'e'*8192)",
    ] {
        let (dir, effect) = fixture(script);
        assert!(run(&effect).await.unwrap().incomplete);
        assert_reaped(&dir).await;
    }
}

#[tokio::test]
async fn repeated_success_error_and_cancel_leave_no_children() {
    for _ in 0..4 {
        for code in [0, 2] {
            let (dir, effect) = fixture(&format!(
                "import os,sys\nopen('pid','w').write(str(os.getpid()))\nprint('entry')\nsys.exit({code})"
            ));
            let result = run(&effect).await;
            assert_eq!(result.is_ok(), code == 0);
            assert_reaped(&dir).await;
        }
        let (dir, effect) =
            fixture("import os,time\nopen('pid','w').write(str(os.getpid()))\ntime.sleep(60)");
        let mut task = tokio::spawn(async move { effect.find(request("*")).await });
        ready_or_result(&dir, &mut task).await;
        task.abort();
        let _ = task.await;
        assert_reaped(&dir).await;
    }
}

#[tokio::test]
async fn shared_path_resolution_and_external_roots_are_preserved() {
    let workspace = TempDir::new().unwrap();
    let external = TempDir::new().unwrap();
    std::fs::create_dir(workspace.path().join("space dir")).unwrap();
    std::fs::write(workspace.path().join("space dir/local.txt"), "").unwrap();
    std::fs::write(external.path().join("external.txt"), "").unwrap();
    let effect = FdFindPaths::new(
        Arc::new(workspace.path().into()),
        Arc::new(Sandbox::new(Some(workspace.path().into()))),
    );
    for path in [
        "@space dir".to_string(),
        "space\u{a0}dir".to_string(),
        format!("{}/", external.path().display()),
    ] {
        let mut req = request("*.txt");
        req.path = path;
        let output = effect.find(req).await.unwrap();
        assert_eq!(output.entries.len(), 1);
        assert!(matches!(
            output.entries[0].as_str(),
            "local.txt" | "external.txt"
        ));
    }
}

#[tokio::test]
async fn injected_wait_failure_reaps_and_returns_io() {
    let (dir, effect) =
        fixture("import os,time\nopen('pid','w').write(str(os.getpid()))\ntime.sleep(60)");
    let mut child = effect.command(&request("*"), dir.path()).spawn().unwrap();
    ready_pid(&dir).await;
    let result = finish_wait(
        &mut child,
        Err(std::io::Error::other("injected wait failure")),
    )
    .await;
    assert!(
        matches!(result, Err(FindError::Io(message)) if message.contains("injected wait failure"))
    );
    assert_reaped(&dir).await;
}

#[test]
fn classification_complete_status_stdout_stderr_matrix() {
    for code in [Some(0), Some(1), Some(2), Some(3), None] {
        for stdout in [b"".as_slice(), b"entry".as_slice()] {
            for stderr in [b"".as_slice(), b"diagnostic".as_slice()] {
                let result = classify(code, stdout, stderr, Path::new("/ws"), 1000);
                match (code, stdout) {
                    (Some(0 | 1), _) => assert!(!result.unwrap().incomplete),
                    (Some(2), _) | (_, b"") => assert!(matches!(result, Err(FindError::Search(_)))),
                    _ => {
                        let result = result.unwrap();
                        assert!(result.incomplete);
                        assert_eq!(result.entries, ["entry"]);
                        assert!(result.diagnostic.is_some());
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn injected_read_failure_reaps_and_returns_io() {
    let (dir, effect) =
        fixture("import os,time\nopen('pid','w').write(str(os.getpid()))\ntime.sleep(60)");
    let mut child = effect.command(&request("*"), dir.path()).spawn().unwrap();
    ready_pid(&dir).await;
    let result = finish_read(
        &mut child,
        Err(std::io::Error::other("injected read failure")),
    )
    .await;
    assert!(
        matches!(result, Err(FindError::Io(message)) if message.contains("injected read failure"))
    );
    assert_reaped(&dir).await;
}

#[test]
fn normalization_whitespace_multibyte_and_long_prefix_bounds() {
    assert!(normalize_output(b"  \n\n", Path::new("/ws"), false).is_empty());
    assert_eq!(
        normalize_output(b"one\n  \n\ntwo", Path::new("/ws"), false),
        ["one", "  ", "two"]
    );
    let bytes = "日本\n途中".as_bytes();
    assert_eq!(
        normalize_output(&bytes[..bytes.len() - 1], Path::new("/ws"), true),
        ["日本"]
    );
    let root = format!("/{}", "long".repeat(3000));
    let raw = format!("{root}/file\n{root}/partial");
    let output = output(raw.as_bytes(), b"", Path::new(&root), 1000, true);
    assert_eq!(output.entries, ["file"]);
    assert!(output.incomplete);
}

#[test]
fn rewrite_and_option_looking_patterns_are_explicit_argv() {
    let effect = FdFindPaths::new(Arc::new(PathBuf::from("/ws")), Arc::new(Sandbox::new(None)));
    for (pattern, rewritten, full) in [
        ("*.rs", "*.rs", false),
        ("src/*.rs", "**/src/*.rs", true),
        ("./src/*.rs", "**/src/*.rs", true),
        ("/src/*.rs", "**/src/*.rs", true),
        ("**/*.rs", "**/*.rs", true),
        ("../*.rs", "**/../*.rs", true),
        ("-option", "-option", false),
    ] {
        let command = effect.command(&request(pattern), Path::new("/ws"));
        let args: Vec<_> = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect();
        assert_eq!(args.contains(&"--full-path"), full);
        assert_eq!(&args[args.len() - 3..], ["--", rewritten, "/ws"]);
    }
}

async fn ready_or_result(
    dir: &TempDir,
    task: &mut tokio::task::JoinHandle<Result<FindOutput, FindError>>,
) {
    tokio::select! {
        _ = ready_pid(dir) => {},
        result = task => panic!("fixture exited before readiness: {result:?}"),
    }
}

#[cfg(target_os = "linux")]
fn runtime_destruction_reaps(mut builder: tokio::runtime::Builder) {
    for preparation in ["", "os.close(1)", "os.close(1); os.close(2)"] {
        let (dir, effect) = fixture(&format!(
            "import os,time\n{preparation}\nopen('pid','w').write(str(os.getpid()))\ntime.sleep(60)"
        ));
        let runtime = builder.enable_all().build().unwrap();
        let mut invocation = effect.find(request("*"));
        let pid = runtime.block_on(async {
            tokio::select! {
                result = &mut invocation => panic!("fixture exited early: {result:?}"),
                pid = ready_pid(&dir) => pid,
            }
        });
        // Retain the invocation across runtime destruction, then cancel it with
        // no caller runtime left to drive cleanup.
        drop(runtime);
        drop(invocation);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let process = PathBuf::from(format!("/proc/{pid}"));
        while process.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let disappeared = matches!(process.try_exists(), Ok(false));
        let mut status = 0;
        // WNOHANG cannot block the test.
        // SAFETY: valid status pointer; targets only our identified fixture child.
        let waited = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) };
        let error = std::io::Error::last_os_error();
        if waited == 0 {
            // SAFETY: cleanup targets only the still-owned fixture PID.
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
                libc::waitpid(pid as libc::pid_t, &mut status, 0);
            }
        }
        assert!(
            disappeared && waited == -1 && error.raw_os_error() == Some(libc::ECHILD),
            "runtime shutdown must reap fd: pid={pid}, preparation={preparation:?}, disappeared={disappeared}, waitpid={waited}, error={error}; waitpid == pid proves test had to reap a zombie"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn current_thread_runtime_destruction_reaps_fd() {
    runtime_destruction_reaps(tokio::runtime::Builder::new_current_thread());
}

#[cfg(target_os = "linux")]
#[test]
fn multi_thread_runtime_destruction_reaps_fd() {
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder.worker_threads(2);
    runtime_destruction_reaps(builder);
}

#[tokio::test]
async fn two_live_calls_keep_children_workspaces_and_arguments_isolated() {
    for shared in [true, false] {
        let script = "import os,sys,time,json\npattern=sys.argv[-2]\nopen(pattern+'.args','w').write(json.dumps(sys.argv[1:]))\nopen(pattern+'.pid','w').write(str(os.getpid()))\nwhile not os.path.exists(pattern+'.release'): time.sleep(0.01)\nprint(os.path.join(sys.argv[-1],pattern))";
        let (first_dir, first) = fixture(script);
        let (second_dir, second) = fixture(script);
        let first = Arc::new(first);
        let second = if shared {
            first.clone()
        } else {
            Arc::new(second)
        };
        let second_root = if shared {
            first_dir.path()
        } else {
            second_dir.path()
        };
        let first_task = tokio::spawn(async move { first.find(request("first")).await });
        let second_task = tokio::spawn(async move {
            let mut req = request("second");
            req.limit = 7;
            second.find(req).await
        });
        let first_pid = named_ready_pid(first_dir.path(), "first").await;
        let second_pid = named_ready_pid(second_root, "second").await;
        assert_ne!(first_pid, second_pid);
        first_task.abort();
        let _ = first_task.await;
        timeout(Duration::from_secs(5), async {
            while Path::new(&format!("/proc/{first_pid}")).exists() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("cancelled invocation must be reaped");
        assert!(Path::new(&format!("/proc/{second_pid}")).exists());
        let status = std::fs::read_to_string(format!("/proc/{second_pid}/status")).unwrap();
        assert!(status.lines().any(|line| {
            line.starts_with("State:") && (line.contains("sleeping") || line.contains("running"))
        }));
        let args: Vec<String> = serde_json::from_str(
            &std::fs::read_to_string(second_root.join("second.args")).unwrap(),
        )
        .unwrap();
        assert_eq!(args[5], "7");
        assert_eq!(args[args.len() - 2], "second");
        assert_eq!(
            args.last().unwrap(),
            &second_root.join(".").to_string_lossy()
        );
        std::fs::write(second_root.join("second.release"), "").unwrap();
        let result = timeout(Duration::from_secs(5), second_task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(result.entries, ["second"]);
        assert!(matches!(
            Path::new(&format!("/proc/{second_pid}")).try_exists(),
            Ok(false)
        ));
    }
}

async fn named_ready_pid(root: &Path, name: &str) -> u32 {
    timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(text) = tokio::fs::read_to_string(root.join(format!("{name}.pid"))).await {
                if let Ok(pid) = text.parse() {
                    return pid;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("both invocations must signal readiness")
}
