//! #2206: the official Docker adapter's `inspect.sh` and `kill.sh` take a
//! container for gone only on the runtime's own word. A runtime that
//! cannot be asked (socket down, storage lock) is no answer: inspect fails
//! (the harness then treats the container as unknown), and a kill fails
//! before it removes the state directory. A runtime that says "no such
//! container" is an answer: dead, and removed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn script(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("scripts/container-runtime/docker")
        .join(name)
}

/// How the fake runtime answers every call.
#[derive(Clone, Copy)]
enum Runtime {
    /// It cannot be reached.
    Unreachable,
    /// It says the container does not exist (Podman's, then Docker's words).
    NoSuchPodman,
    NoSuchDocker,
    /// It answers: the container exists in this state (`exited`,
    /// `running`, `paused`, `created`, ...).
    State(&'static str),
}

fn fake_cli(dir: &Path, runtime: Runtime) -> (PathBuf, PathBuf) {
    let log = dir.join("cli.log");
    let body = match runtime {
        Runtime::Unreachable => {
            "echo 'Error: unable to connect to Podman socket: connection refused' >&2\nexit 125"
        }
        Runtime::NoSuchPodman => "echo 'Error: no such object: \"quecto-env-x\"' >&2\nexit 125",
        Runtime::NoSuchDocker => {
            "echo 'Error response from daemon: No such container: quecto-env-x' >&2\nexit 1"
        }
        Runtime::State(_) => "",
    };
    let body = match runtime {
        // `{{.State.Status}}` alone, or with the exit code and OOM flag.
        Runtime::State(state) => format!(
            "case \"$1\" in inspect) case \"$3\" in *ExitCode*) echo '{state} 0 false';; *) echo '{state}';; esac;; esac\nexit 0"
        ),
        _ => body.to_string(),
    };
    let cli = dir.join("fake-cli");
    fs::write(
        &cli,
        format!(
            "#!/usr/bin/env bash\nprintf '%s\\n' \"$*\" >>'{}'\n{body}\n",
            log.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&cli, fs::Permissions::from_mode(0o755)).unwrap();
    }
    (cli, log)
}

/// A state directory holding `env-x` and its container name.
fn state(dir: &Path) -> PathBuf {
    let state = dir.join("state");
    fs::create_dir_all(state.join("env-x/workspace/repo")).unwrap();
    fs::write(state.join("env-x/container"), "quecto-env-x\n").unwrap();
    fs::write(
        state.join("env-x/workspace/repo/work.txt"),
        "unpushed work\n",
    )
    .unwrap();
    state
}

fn run(name: &str, cli: &Path, state: &Path, args: &[&str]) -> std::process::Output {
    Command::new(script(name))
        .arg("--state-dir")
        .arg(state)
        .args(args)
        .env("QUECTO_CONTAINER_CLI", cli)
        .env("QUECTO_CONTAINER_ENVIRONMENT_ID", "env-x")
        .output()
        .expect("run the script")
}

fn status_of(output: &std::process::Output) -> String {
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&output.stdout)));
    value["status"].as_str().unwrap_or_default().to_string()
}

#[test]
fn inspect_fails_when_the_runtime_cannot_be_asked() {
    let temp = tempfile::tempdir().unwrap();
    let (cli, _) = fake_cli(temp.path(), Runtime::Unreachable);
    let state = state(temp.path());
    let output = run("inspect.sh", &cli, &state, &[]);
    assert!(!output.status.success(), "never a dead guess");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("whether it exists is unknown"), "{stderr}");
    assert!(stderr.contains("connection refused"), "{stderr}");
    // Its state directory gone does not make the runtime's silence an answer.
    fs::remove_dir_all(state.join("env-x")).unwrap();
    assert!(!run("inspect.sh", &cli, &state, &[]).status.success());
}

#[test]
fn inspect_reports_dead_on_the_runtimes_own_no_such_container() {
    for runtime in [Runtime::NoSuchPodman, Runtime::NoSuchDocker] {
        let temp = tempfile::tempdir().unwrap();
        let (cli, _) = fake_cli(temp.path(), runtime);
        let state = state(temp.path());
        let output = run("inspect.sh", &cli, &state, &[]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(status_of(&output), "dead");
        fs::remove_dir_all(state.join("env-x")).unwrap();
        let output = run("inspect.sh", &cli, &state, &[]);
        assert!(output.status.success());
        assert_eq!(status_of(&output), "dead");
    }
    for (state, status) in [
        ("exited", "dead"),
        ("dead", "dead"),
        ("stopped", "dead"),
        ("running", "running"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let (cli, _) = fake_cli(temp.path(), Runtime::State(state));
        let dir = self::state(temp.path());
        let output = run("inspect.sh", &cli, &dir, &[]);
        assert_eq!(status_of(&output), status, "{state}");
        fs::remove_dir_all(dir.join("env-x")).unwrap();
        let output = run("inspect.sh", &cli, &dir, &[]);
        assert_eq!(status_of(&output), status, "{state}, no state dir");
    }
}

/// #2206 round 2: only `running` is running and only `exited`/`dead` is
/// dead; a paused, created or stopping container is neither — unknown.
#[test]
fn inspect_fails_for_a_container_neither_running_nor_exited() {
    for state in ["paused", "created", "stopping", "restarting"] {
        let temp = tempfile::tempdir().unwrap();
        let (cli, _) = fake_cli(temp.path(), Runtime::State(state));
        let dir = self::state(temp.path());
        let output = run("inspect.sh", &cli, &dir, &[]);
        assert!(!output.status.success(), "{state}: never a dead guess");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("neither running nor exited"),
            "{state}"
        );
        fs::remove_dir_all(dir.join("env-x")).unwrap();
        assert!(
            !run("inspect.sh", &cli, &dir, &[]).status.success(),
            "{state}"
        );
    }
}

/// #2206 round 2: the scripts need no writable temporary directory.
#[test]
fn the_scripts_need_no_writable_tmpdir() {
    let temp = tempfile::tempdir().unwrap();
    let (cli, _) = fake_cli(temp.path(), Runtime::NoSuchPodman);
    let dir = self::state(temp.path());
    for (name, args) in [
        ("inspect.sh", &[][..]),
        ("kill.sh", &["--op", "cleanup"][..]),
    ] {
        let output = Command::new(script(name))
            .arg("--state-dir")
            .arg(&dir)
            .args(args)
            .env("QUECTO_CONTAINER_CLI", &cli)
            .env("QUECTO_CONTAINER_ENVIRONMENT_ID", "env-x")
            .env("TMPDIR", "/nonexistent/unwritable")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(!dir.join("env-x").exists());
}

/// A "no such container" about some other container is not an answer
/// about this one.
#[test]
fn a_no_such_answer_about_another_container_is_no_answer() {
    let temp = tempfile::tempdir().unwrap();
    let cli = temp.path().join("fake-cli");
    fs::write(
        &cli,
        "#!/usr/bin/env bash\necho 'Error: no such container quecto-other' >&2\nexit 125\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&cli, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let dir = self::state(temp.path());
    assert!(!run("inspect.sh", &cli, &dir, &[]).status.success());
    assert!(
        !run("kill.sh", &cli, &dir, &["--op", "cleanup"])
            .status
            .success()
    );
    assert!(dir.join("env-x/workspace/repo/work.txt").exists());
}

#[test]
fn a_kill_that_cannot_remove_the_container_keeps_the_state_directory() {
    for op in ["kill", "cleanup"] {
        let temp = tempfile::tempdir().unwrap();
        let (cli, log) = fake_cli(temp.path(), Runtime::Unreachable);
        let state = state(temp.path());
        let output = run("kill.sh", &cli, &state, &["--op", op]);
        assert!(!output.status.success(), "{op}: the failure is the kill's");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("the state directory is kept"), "{stderr}");
        assert!(
            state.join("env-x/workspace/repo/work.txt").exists(),
            "{op}: the work survives"
        );
        assert!(fs::read_to_string(log).unwrap().contains("rm -f"));
        // With the directory already gone, the container is still owed.
        fs::remove_dir_all(state.join("env-x")).unwrap();
        assert!(!run("kill.sh", &cli, &state, &["--op", op]).status.success());
    }
}

#[test]
fn a_kill_of_a_container_the_runtime_says_is_gone_removes_everything() {
    for runtime in [Runtime::NoSuchPodman, Runtime::NoSuchDocker] {
        for op in ["kill", "cleanup"] {
            let temp = tempfile::tempdir().unwrap();
            let (cli, _) = fake_cli(temp.path(), runtime);
            let state = state(temp.path());
            let output = run("kill.sh", &cli, &state, &["--op", op]);
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(!state.join("env-x").exists(), "{op}: state removed");
            assert!(run("kill.sh", &cli, &state, &["--op", op]).status.success());
        }
    }
}
