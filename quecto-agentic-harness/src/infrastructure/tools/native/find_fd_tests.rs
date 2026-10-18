use super::*;
use crate::infrastructure::test_support::executable::write_executable;
use tempfile::TempDir;
use tokio::time::{Duration, timeout};

fn request(pattern: &str) -> FindPathsRequest {
    FindPathsRequest {
        pattern: pattern.into(),
        path: ".".into(),
        limit: 1000,
        kind: None,
    }
}

/// A search root shown as the workspace itself.
pub(super) fn at(root: &str) -> SearchRoot {
    SearchRoot {
        searched: PathBuf::from(root),
        shown: String::new(),
        workspace: PathBuf::from("/"),
        kind: None,
        skipped_vcs_dir: None,
    }
}

/// A fake fd's first lines: it records its pid (the one the tests watch
/// be reaped), and the 8 KiB chunk it writes under pipe pressure.
pub(super) const PID: &str = "printf %s $$ > pid";
const CHUNK: &str = "chunk=x\ni=0\nwhile [ $i -lt 13 ]; do chunk=$chunk$chunk; i=$((i+1)); done";
/// A fake fd that records its pid and sleeps as that very process (`exec`),
/// so the pid the tests watch is the one the call must reap.
pub(super) const SLEEPER: &str = "printf %s $$ > pid\nexec sleep 60";
/// The streams a fake fd closes before it sleeps: none, stdout, both.
const CLOSED_STREAMS: [&str; 3] = ["", "exec 1>&-", "exec 1>&- 2>&-"];

/// A fake fd's recorded argv: each argument NUL-terminated, so an
/// argument may hold any other byte, a newline included.
fn recorded_argv(path: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(path).unwrap();
    let body = text
        .strip_suffix('\0')
        .expect("every argument is NUL-terminated");
    body.split('\0').map(str::to_owned).collect()
}

/// A fake fd running the POSIX shell `script` in a fresh directory.
pub(super) fn fixture(script: &str) -> (TempDir, FdFindPaths) {
    let dir = TempDir::new().unwrap();
    let binary = dir.path().join("fake-fd");
    write_executable(&binary, format!("#!/bin/sh\n{script}\n"));
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
        for output in ["", "entry\\0"] {
            let (_dir, effect) = fixture(&format!(
                "printf '{output}'\nprintf diagnostic >&2\nexit {code}"
            ));
            let result = run(&effect).await;
            // fd exits 1 on an error (#2164): like any other failure, it
            // is an error, or an incomplete result when fd found some first.
            match (code, output.is_empty()) {
                (0, _) => assert!(
                    result.is_ok(),
                    "code={code} output={output:?} result={result:?}"
                ),
                (1 | 3, false) => assert!(result.unwrap().incomplete),
                _ => assert!(matches!(result, Err(FindError::Search(_)))),
            }
        }
    }
}

#[tokio::test]
async fn either_pipe_pressure_stops_and_reaps() {
    for stream in ["stdout", "stderr"] {
        let redirect = match stream {
            "stdout" => "",
            _ => ">&2",
        };
        let (dir, effect) = fixture(&format!(
            "{PID}\n{CHUNK}\nwhile :; do printf %s \"$chunk\" {redirect}; done"
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

pub(super) async fn assert_reaped(dir: &TempDir) {
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
    for preparation in CLOSED_STREAMS {
        let (dir, effect) = fixture(&format!("{preparation}\n{SLEEPER}"));
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
        b"/ws/file\0/ws2/other\0/ws/dir/\0partial",
        &at("/ws/"),
        true,
    );
    assert_eq!(output, ["file", "/ws2/other", "dir/"]);
    assert_eq!(normalize_output(b"/file\0", &at("/"), false), ["file"]);
    assert_eq!(
        normalize_output(b"\xff\0last", &at("/ws"), false),
        ["\u{fffd}", "last"]
    );
}

#[tokio::test]
async fn argv_cwd_and_glob_are_literal() {
    let (dir, effect) = fixture("printf '%s\\0' \"$@\" > argv\nprintf %s \"$(pwd -P)\" > cwd");
    let mut req = request("./src/*.rs");
    req.limit = 7;
    effect.find(req).await.unwrap();
    let args = recorded_argv(&dir.path().join("argv"));
    assert_eq!(
        &args[..18],
        [
            "--glob",
            "--color=never",
            "--hidden",
            "--no-require-git",
            // NUL-separated records (#2200 review 3).
            "--print0",
            "--max-results",
            // One past the limit (#2176 review).
            "8",
            // VCS internals are skipped below the search root (#2199).
            "--exclude",
            ".git",
            "--exclude",
            ".hg",
            "--exclude",
            ".svn",
            "--exclude",
            ".jj",
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
    // An unfinished record is bounded by RECORD_MAX: longer is no path.
    for count in [RECORD_MAX, RECORD_MAX + 1] {
        let (_dir, effect) = fixture(&format!("head -c {count} /dev/zero | tr -c x x"));
        let output = run(&effect).await.unwrap();
        if count <= RECORD_MAX {
            assert_eq!(output.entries[0].len(), count);
            assert!(!output.incomplete);
        } else {
            assert!(output.entries.is_empty());
            assert!(output.incomplete);
        }
    }
    // Kept entries are bounded by the byte cap: past it the search is
    // incomplete, and what is kept fits.
    let (_dir, effect) = fixture(&format!(
        "y=$(head -c 999 /dev/zero | tr -c y y)\ni=0\nwhile [ $i -lt {} ]; do printf '%s\\0' \"$y\"; i=$((i+1)); done",
        STDOUT_CAP / 1000 + 5
    ));
    let output = run(&effect).await.unwrap();
    assert!(output.incomplete);
    assert_eq!(output.entries.len(), STDOUT_CAP / 1000);
}

#[tokio::test]
async fn signal_status_with_partial_output_is_incomplete() {
    for data in ["", "entry\\0"] {
        let (_dir, effect) = fixture(&format!("printf '{data}'\nkill -TERM $$"));
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
    let (cancel_dir, cancel) = fixture(SLEEPER);
    let (_success_dir, success) = fixture("printf 'other-entry\\0'");
    let mut task = tokio::spawn(async move { cancel.find(request("*")).await });
    ready_or_result(&cancel_dir, &mut task).await;
    task.abort();
    let _ = task.await;
    assert_eq!(run(&success).await.unwrap().entries, ["other-entry"]);
    assert_reaped(&cancel_dir).await;
}

#[tokio::test]
async fn missing_binary_and_fd_status_one_are_errors() {
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
    // fd reports an invalid root or glob with status 1 (#2164): the agent is
    // told what fd said, never "no files found".
    for (path, pattern, said) in [
        ("missing", "*", "is not a directory"),
        ("file", "*", "is not a directory"),
        (".", "[", "unclosed character class"),
    ] {
        let mut req = request(pattern);
        req.path = path.into();
        let result = effect.find(req).await;
        assert!(
            matches!(&result, Err(FindError::Search(message)) if message.contains(said)),
            "{path} {pattern}: {result:?}"
        );
    }
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
    let (dir, effect) = fixture("printf yes > spawned");
    drop(effect.find(request("*")));
    assert!(!dir.path().join("spawned").exists());
}

#[tokio::test]
async fn both_pipes_and_closed_stdout_pressure_are_bounded() {
    for script in [
        format!("{PID}\n{CHUNK}\nwhile :; do printf %s \"$chunk\"; printf %s \"$chunk\" >&2; done"),
        format!("{PID}\n{CHUNK}\nexec 1>&-\nwhile :; do printf %s \"$chunk\" >&2; done"),
    ] {
        let (dir, effect) = fixture(&script);
        assert!(run(&effect).await.unwrap().incomplete);
        assert_reaped(&dir).await;
    }
}

#[tokio::test]
async fn repeated_success_error_and_cancel_leave_no_children() {
    for _ in 0..4 {
        for code in [0, 2] {
            let (dir, effect) = fixture(&format!("{PID}\nprintf 'entry\\0'\nexit {code}"));
            let result = run(&effect).await;
            assert_eq!(result.is_ok(), code == 0);
            assert_reaped(&dir).await;
        }
        let (dir, effect) = fixture(SLEEPER);
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
    // Inside the workspace a path is shown relative to it; outside, whole
    // (#2203), as grep shows them.
    let outside = std::fs::canonicalize(external.path()).unwrap();
    for (path, shown) in [
        ("@space dir".to_string(), "space dir/local.txt".to_string()),
        (
            "space\u{a0}dir".to_string(),
            "space dir/local.txt".to_string(),
        ),
        (
            format!("{}/", external.path().display()),
            format!("{}/external.txt", outside.display()),
        ),
    ] {
        let mut req = request("*.txt");
        req.path = path;
        let output = effect.find(req).await.unwrap();
        assert_eq!(output.entries, [shown]);
    }
}

#[tokio::test]
async fn injected_wait_failure_reaps_and_returns_io() {
    let (dir, effect) = fixture(SLEEPER);
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
                let result = classify(code, stdout, stderr, &at("/ws"), 1000);
                // fd exits 0 whether or not anything matched, and 1 on an
                // error (#2164): never a clean empty result.
                match (code, stdout) {
                    (Some(0), _) => assert!(!result.unwrap().incomplete),
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
    let (dir, effect) = fixture(SLEEPER);
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
    // Only empty lines are skipped: fd never prints a blank path, and a
    // name of spaces is still a name.
    assert_eq!(normalize_output(b"  \0\0", &at("/ws"), false), ["  "]);
    assert_eq!(
        normalize_output(b"one\0  \0\0two", &at("/ws"), false),
        ["one", "  ", "two"]
    );
    let bytes = "日本\0途中".as_bytes();
    assert_eq!(
        normalize_output(&bytes[..bytes.len() - 1], &at("/ws"), true),
        ["日本"]
    );
    let root = format!("/{}", "long".repeat(3000));
    let raw = format!("{root}/file\0{root}/partial");
    let output = output(raw.as_bytes(), b"", &at(&root), 1000, true);
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
    for preparation in CLOSED_STREAMS {
        let (dir, effect) = fixture(&format!("{preparation}\n{SLEEPER}"));
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
        // The pattern is the second-to-last argument, the root the last.
        // The pid it records is the process the test samples, and it
        // waits for its release without forking: blocked opening and
        // reading a FIFO, never polling through child `sleep`s (a shell that
        // vforks them, dash, shows `D` while one starts).
        let script = "prev=\nlast=\nfor a in \"$@\"; do prev=$last; last=$a; done\n\
                      printf '%s\\0' \"$@\" > \"$prev.args\"\nmkfifo \"$prev.release\"\n\
                      printf %s $$ > \"$prev.pid\"\nread -r _ < \"$prev.release\" || :\n\
                      printf '%s/%s\\0' \"$last\" \"$prev\"";
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
        let state = status
            .lines()
            .find(|line| line.starts_with("State:"))
            .unwrap_or_default();
        assert!(
            state.contains("sleeping") || state.contains("running"),
            "the second call's fd is untouched: {state:?}, {}",
            process_tree(second_pid)
        );
        let args = recorded_argv(&second_root.join("second.args"));
        assert_eq!(args[6], "8", "one past the limit");
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

/// `pid`'s parent, process group and session, then its parent's, up to
/// this test process: for a state assertion's failure report.
fn process_tree(pid: u32) -> String {
    let mut links = Vec::new();
    let mut current = pid;
    for _ in 0..8 {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{current}/stat")) else {
            links.push(format!("{current}: gone"));
            break;
        };
        let (name, fields) = stat.rsplit_once(") ").unwrap_or(("?", ""));
        let fields: Vec<&str> = fields.split_whitespace().collect();
        let field = |index: usize| fields.get(index).copied().unwrap_or("?");
        links.push(format!(
            "{current} {}) state {} ppid {} pgid {} sid {}",
            name,
            field(0),
            field(1),
            field(2),
            field(3)
        ));
        match field(1).parse::<u32>() {
            Ok(parent) if parent > 1 && current != std::process::id() => current = parent,
            _ => break,
        }
    }
    links.join(" <- ")
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

/// #2164: fd's own error (a malformed glob, exit 1) is reported, never
/// taken for "no files found".
#[test]
fn an_fd_error_is_reported_not_taken_for_no_matches() {
    let stderr = b"[fd error]: error parsing glob '[': unclosed character class; missing ']'";
    let result = classify(Some(1), b"", stderr, &at("/ws"), 1000);
    assert!(
        matches!(&result, Err(FindError::Search(message)) if message.contains("unclosed character class")),
        "{result:?}"
    );
}

/// What fd returns is shown sorted, whatever order its threads found it in.
#[test]
fn entries_are_sorted() {
    let result = classify(Some(0), b"/ws/b\0/ws/a/z\0/ws/a\0", b"", &at("/ws"), 1000).unwrap();
    assert_eq!(result.entries, ["a", "a/z", "b"]);
}

/// #2176 review: exactly `limit` matches is complete; one past it (fd is
/// asked for limit + 1) means more exist, and the listing keeps `limit`.
#[test]
fn the_limit_is_known_from_one_extra_entry() {
    let exact = classify(Some(0), b"/ws/a\0/ws/b\0", b"", &at("/ws"), 2).unwrap();
    assert!(!exact.result_limit_reached);
    assert_eq!(exact.entries, ["a", "b"]);
    let more = classify(Some(0), b"/ws/c\0/ws/a\0/ws/b\0", b"", &at("/ws"), 2).unwrap();
    assert!(more.result_limit_reached);
    assert_eq!(more.entries, ["a", "b"]);
}

/// #2176 review: output cut at the stdout cap is an arbitrary subset too.
#[test]
fn a_capped_read_is_an_arbitrary_subset() {
    let capped = output(b"/ws/b\0/ws/a\0/ws/c", b"", &at("/ws"), 1000, true);
    assert!(capped.incomplete);
    assert!(capped.result_limit_reached);
    assert_eq!(capped.entries, ["a", "b"], "the cut last line is dropped");
}
