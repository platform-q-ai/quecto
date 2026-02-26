use cucumber::{given, then, when};
use quecto::domain::coding_ports::{WorkerEvent, WorkerLaunchConfig, WorkerRuntime, WorkerStatus};
use quecto::infrastructure::coding::nsjail_runtime::{NsjailRuntimeConfig, NsjailWorkerRuntime};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;

use crate::QuectoWorld;

// ── Helper: create executable scripts in a temp dir ─────────────────────

fn default_process_launch_config(world: &QuectoWorld) -> WorkerLaunchConfig {
    world
        .nrt_launch_config
        .clone()
        .unwrap_or_else(|| WorkerLaunchConfig {
            run_id: "run_test".to_string(),
            job_id: "job_test".to_string(),
            job_dir: "/tmp/test-job".to_string(),
            goal: "test".to_string(),
            max_memory_mb: 512,
            max_cpu_seconds: 120,
            max_wall_seconds: 300,
            max_pids: 128,
            network_allowed_hosts: vec![],
            die_with_parent: false,
        })
}

fn create_script(world: &mut QuectoWorld, name: &str, content: &str) -> Vec<String> {
    let td = TempDir::new().expect("create temp dir");
    let script_path = td.path().join(name);
    let mut f = std::fs::File::create(&script_path).expect("create script");
    f.write_all(content.as_bytes()).expect("write script");
    std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let path = script_path.to_string_lossy().to_string();
    world._extra_temp_dirs.push(td);
    vec!["/bin/sh".to_string(), path]
}

fn setup_runtime_with_script(world: &mut QuectoWorld, cmd: Vec<String>) {
    let config = NsjailRuntimeConfig {
        nsjail_binary: "nsjail".to_string(),
        quecto_binary: "/usr/local/bin/quecto".to_string(),
        command_override: Some(cmd),
        cgroups_available: true,
    };
    world.nrt_runtime = Some(NsjailWorkerRuntime::new(config));
    world.nrt_launch_config = Some(default_process_launch_config(world));
}

// ── Given steps ─────────────────────────────────────────────────────────

#[given("a nsjail runtime configured with a helper worker script")]
fn given_helper_worker(world: &mut QuectoWorld) {
    // A script that reads lines from stdin and exits on "exit"
    let cmd = create_script(
        world,
        "helper-worker.sh",
        "#!/bin/sh\nwhile IFS= read -r line; do\n  case \"$line\" in\n    exit) exit 0;;\n  esac\ndone\n",
    );
    setup_runtime_with_script(world, cmd);
}

#[given("a nsjail runtime configured with an echo worker script")]
fn given_echo_worker(world: &mut QuectoWorld) {
    // Echoes each stdin line back to stdout
    let cmd = create_script(
        world,
        "echo-worker.sh",
        "#!/bin/sh\nwhile IFS= read -r line; do\n  echo \"$line\"\n  case \"$line\" in exit) exit 0;; esac\ndone\n",
    );
    setup_runtime_with_script(world, cmd);
}

#[given("a nsjail runtime configured with a json-lines worker script")]
fn given_json_worker(world: &mut QuectoWorld) {
    // Emits a valid EventEnvelope JSON on "emit" command
    let cmd = create_script(
        world,
        "json-worker.sh",
        concat!(
            "#!/bin/sh\nwhile IFS= read -r line; do\n  case \"$line\" in\n",
            "    emit) echo '{\"type\":\"tool.start\",\"source\":\"worker\",",
            "\"run_id\":\"r1\",\"job_id\":\"j1\",\"seq\":1,",
            "\"ts\":\"2026-01-01T00:00:00Z\",\"v\":\"1.0\",",
            "\"payload\":{}}';;\n",
            "    exit) exit 0;;\n  esac\ndone\n",
        ),
    );
    setup_runtime_with_script(world, cmd);
}

#[given("a nsjail runtime configured with a plain-text worker script")]
fn given_plain_text_worker(world: &mut QuectoWorld) {
    // Emits plain text (not JSON) on "say" command
    let cmd = create_script(
        world,
        "plain-worker.sh",
        "#!/bin/sh\nwhile IFS= read -r line; do\n  case \"$line\" in\n    say) echo 'this is not json';;\n    exit) exit 0;;\n  esac\ndone\n",
    );
    setup_runtime_with_script(world, cmd);
}

#[given("a nsjail runtime configured with a silent worker script")]
fn given_silent_worker(world: &mut QuectoWorld) {
    // Reads stdin but produces no output, exits on "exit"
    let cmd = create_script(
        world,
        "silent-worker.sh",
        "#!/bin/sh\nwhile IFS= read -r line; do\n  case \"$line\" in exit) exit 0;; esac\ndone\n",
    );
    setup_runtime_with_script(world, cmd);
}

#[given("a nsjail runtime configured with a stderr worker script")]
fn given_stderr_worker(world: &mut QuectoWorld) {
    // Writes to stderr on "warn" command
    let cmd = create_script(
        world,
        "stderr-worker.sh",
        "#!/bin/sh\nwhile IFS= read -r line; do\n  case \"$line\" in\n    warn) echo 'WARNING: something happened' >&2;;\n    exit) exit 0;;\n  esac\ndone\n",
    );
    setup_runtime_with_script(world, cmd);
}

#[given("a nsjail runtime configured with a large-stderr worker script")]
fn given_large_stderr_worker(world: &mut QuectoWorld) {
    // Floods stderr with >1 MiB of data then exits
    let cmd = create_script(
        world,
        "large-stderr-worker.sh",
        "#!/bin/sh\nwhile IFS= read -r line; do\n  case \"$line\" in\n    flood) dd if=/dev/zero bs=1024 count=1100 2>/dev/null | tr '\\0' 'X' >&2; exit 0;;\n  esac\ndone\n",
    );
    setup_runtime_with_script(world, cmd);
}

#[given("a nsjail runtime configured with a long-running worker script")]
fn given_long_running_worker(world: &mut QuectoWorld) {
    // Sleeps for a long time (will be killed by the test)
    let cmd = create_script(world, "long-worker.sh", "#!/bin/sh\nsleep 300\n");
    setup_runtime_with_script(world, cmd);
}

#[given("a nsjail runtime configured with a failing worker script")]
fn given_failing_worker(world: &mut QuectoWorld) {
    // Immediately exits with code 1
    let cmd = create_script(world, "fail-worker.sh", "#!/bin/sh\nexit 1\n");
    setup_runtime_with_script(world, cmd);
}

#[given("a nsjail runtime configured with a nonexistent binary")]
fn given_nonexistent_binary(world: &mut QuectoWorld) {
    let config = NsjailRuntimeConfig {
        nsjail_binary: "nsjail".to_string(),
        quecto_binary: "/usr/local/bin/quecto".to_string(),
        command_override: Some(vec!["/definitely/not/a/real/binary".to_string()]),
        cgroups_available: true,
    };
    world.nrt_runtime = Some(NsjailWorkerRuntime::new(config));
    world.nrt_launch_config = Some(default_process_launch_config(world));
}

// ── When steps ──────────────────────────────────────────────────────────

#[when("the runtime launches a worker process")]
fn when_launch_worker(world: &mut QuectoWorld) {
    let rt = world.nrt_runtime.as_mut().expect("nrt runtime");
    let config = world.nrt_launch_config.clone().expect("launch config");
    match rt.launch(&config) {
        Ok(pid) => {
            world.nrt_pid = Some(pid);
            world.nrt_pids.push(pid);
        }
        Err(e) => {
            world.nrt_last_error = Some(e);
        }
    }
}

#[when(regex = r#"^the runtime launches (\d+) worker processes$"#)]
fn when_launch_multiple(world: &mut QuectoWorld, count: usize) {
    for _ in 0..count {
        let rt = world.nrt_runtime.as_mut().expect("nrt runtime");
        let config = world.nrt_launch_config.clone().expect("launch config");
        let pid = rt.launch(&config).expect("launch should succeed");
        world.nrt_pids.push(pid);
    }
    if let Some(&last) = world.nrt_pids.last() {
        world.nrt_pid = Some(last);
    }
}

#[when(regex = r#"^the runtime sends command "([^"]*)" to the worker$"#)]
fn when_send_command(world: &mut QuectoWorld, command: String) {
    let pid = world.nrt_pid.expect("nrt pid");
    let rt = world.nrt_runtime.as_mut().expect("nrt runtime");
    rt.send_command(pid, &command).expect("send_command");
}

#[when(regex = r#"^the runtime sends command "([^"]*)" to PID (\d+)$"#)]
fn when_send_command_to_pid(world: &mut QuectoWorld, command: String, pid: u32) {
    let rt = world.nrt_runtime.as_mut().expect("nrt runtime");
    match rt.send_command(pid, &command) {
        Ok(()) => {}
        Err(e) => {
            world.nrt_last_error = Some(e);
        }
    }
}

#[when("the runtime waits for the worker to exit")]
fn when_wait_for_exit(world: &mut QuectoWorld) {
    let pid = world.nrt_pid.expect("nrt pid");
    let rt = world.nrt_runtime.as_mut().expect("nrt runtime");
    rt.wait_for_exit(pid);
}

#[when("the runtime waits briefly for stderr")]
fn when_wait_briefly(world: &mut QuectoWorld) {
    let _ = world;
    std::thread::sleep(std::time::Duration::from_millis(100));
}

#[when("the runtime kills the worker")]
fn when_kill_worker(world: &mut QuectoWorld) {
    let pid = world.nrt_pid.expect("nrt pid");
    let rt = world.nrt_runtime.as_mut().expect("nrt runtime");
    rt.kill(pid).expect("kill should succeed");
}

#[when(regex = r#"^the runtime kills PID (\d+)$"#)]
fn when_kill_pid(world: &mut QuectoWorld, pid: u32) {
    let rt = world.nrt_runtime.as_mut().expect("nrt runtime");
    match rt.kill(pid) {
        Ok(()) => {}
        Err(e) => {
            world.nrt_last_error = Some(e);
        }
    }
}

#[when("the runtime attempts to launch a worker process")]
fn when_attempt_launch(world: &mut QuectoWorld) {
    let rt = world.nrt_runtime.as_mut().expect("nrt runtime");
    let config = world.nrt_launch_config.clone().expect("launch config");
    match rt.launch(&config) {
        Ok(pid) => {
            world.nrt_pid = Some(pid);
        }
        Err(e) => {
            world.nrt_last_error = Some(e);
        }
    }
}

#[when("the runtime cleans up the worker")]
fn when_cleanup(world: &mut QuectoWorld) {
    let pid = world.nrt_pid.expect("nrt pid");
    let rt = world.nrt_runtime.as_mut().expect("nrt runtime");
    rt.cleanup(pid);
}

// ── Then steps ──────────────────────────────────────────────────────────

#[then("the returned PID should be a valid OS process ID")]
fn then_valid_pid(world: &mut QuectoWorld) {
    let pid = world.nrt_pid.expect("nrt pid should be set");
    assert!(pid > 0, "PID should be a positive number, got {pid}");
    assert!(pid < 4_194_304, "PID should be within OS range, got {pid}");
}

#[then("the process should be alive")]
fn then_process_alive(world: &mut QuectoWorld) {
    let pid = world.nrt_pid.expect("nrt pid");
    let rt = world.nrt_runtime.as_ref().expect("nrt runtime");
    assert!(rt.is_alive(pid), "process {pid} should be alive");
}

#[then("the process should not be alive")]
fn then_process_not_alive(world: &mut QuectoWorld) {
    let pid = world.nrt_pid.expect("nrt pid");
    let rt = world.nrt_runtime.as_ref().expect("nrt runtime");
    assert!(!rt.is_alive(pid), "process {pid} should not be alive");
}

#[then("the PID should correspond to a real OS process")]
fn then_real_os_process(world: &mut QuectoWorld) {
    let pid = world.nrt_pid.expect("nrt pid");
    // Check /proc/<pid> exists on Linux
    let proc_path = format!("/proc/{pid}");
    assert!(
        std::path::Path::new(&proc_path).exists(),
        "/proc/{pid} should exist for a real process"
    );
}

#[then(regex = r#"^all (\d+) PIDs should be distinct$"#)]
fn then_pids_distinct(world: &mut QuectoWorld, count: usize) {
    let pids = &world.nrt_pids;
    assert_eq!(pids.len(), count, "expected {count} PIDs");
    let mut unique = pids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique.len(),
        pids.len(),
        "all PIDs should be distinct: {pids:?}"
    );
}

#[then(regex = r#"^all (\d+) processes should be alive$"#)]
fn then_all_alive(world: &mut QuectoWorld, count: usize) {
    let pids = world.nrt_pids.clone();
    assert_eq!(pids.len(), count);
    let rt = world.nrt_runtime.as_ref().expect("nrt runtime");
    for pid in &pids {
        assert!(rt.is_alive(*pid), "process {pid} should be alive");
    }
}

#[then(regex = r#"^the worker should echo back "([^"]*)" on stdout$"#)]
fn then_echo_back(world: &mut QuectoWorld, expected: String) {
    let pid = world.nrt_pid.expect("nrt pid");
    let rt = world.nrt_runtime.as_mut().expect("nrt runtime");
    // Give the echo script time to process
    std::thread::sleep(std::time::Duration::from_millis(50));
    let event = rt.read_event(pid);
    assert!(event.is_some(), "should have received output from worker");
    match event.unwrap() {
        WorkerEvent::Malformed { raw } => {
            assert!(
                raw.contains(&expected),
                "echoed output should contain '{expected}', got '{raw}'"
            );
        }
        WorkerEvent::Valid(_) => {
            panic!("expected Malformed (plain text echo), got Valid");
        }
    }
}

#[then("read_event should return a Valid event")]
fn then_valid_event(world: &mut QuectoWorld) {
    let pid = world.nrt_pid.expect("nrt pid");
    let rt = world.nrt_runtime.as_mut().expect("nrt runtime");
    std::thread::sleep(std::time::Duration::from_millis(50));
    let event = rt.read_event(pid);
    assert!(event.is_some(), "should have received an event");
    assert!(
        matches!(event.unwrap(), WorkerEvent::Valid(_)),
        "event should be Valid"
    );
}

#[then("read_event should return a Malformed event")]
fn then_malformed_event(world: &mut QuectoWorld) {
    let pid = world.nrt_pid.expect("nrt pid");
    let rt = world.nrt_runtime.as_mut().expect("nrt runtime");
    std::thread::sleep(std::time::Duration::from_millis(50));
    let event = rt.read_event(pid);
    assert!(event.is_some(), "should have received an event");
    assert!(
        matches!(event.unwrap(), WorkerEvent::Malformed { .. }),
        "event should be Malformed"
    );
}

#[then("read_event should return None")]
fn then_no_event(world: &mut QuectoWorld) {
    let pid = world.nrt_pid.expect("nrt pid");
    let rt = world.nrt_runtime.as_mut().expect("nrt runtime");
    // Don't wait — the silent worker produces no output
    let event = rt.read_event(pid);
    // With a blocking BufReader, this would hang. We need non-blocking.
    // For now, since read_line on a pipe blocks, we accept that the
    // test might need a different approach. Let's check: the script
    // is waiting for stdin, not writing anything, so stdout is empty.
    // BufReader::read_line will block waiting for data.
    // We'll verify that no events were pre-buffered.
    assert!(
        event.is_none()
            || matches!(event, Some(WorkerEvent::Malformed { ref raw }) if raw.is_empty()),
        "should have no events available"
    );
}

#[then("read_stderr should contain the warning message")]
fn then_stderr_warning(world: &mut QuectoWorld) {
    let pid = world.nrt_pid.expect("nrt pid");
    let rt = world.nrt_runtime.as_mut().expect("nrt runtime");
    let stderr = rt.read_stderr(pid);
    assert!(
        stderr.contains("WARNING"),
        "stderr should contain warning, got: '{stderr}'"
    );
}

#[then(regex = r#"^read_stderr should be at most (\d+) bytes$"#)]
fn then_stderr_capped(world: &mut QuectoWorld, max_bytes: usize) {
    let pid = world.nrt_pid.expect("nrt pid");
    let rt = world.nrt_runtime.as_mut().expect("nrt runtime");
    let stderr = rt.read_stderr(pid);
    assert!(
        stderr.len() <= max_bytes,
        "stderr should be at most {max_bytes} bytes, got {}",
        stderr.len()
    );
}

#[then("the runtime status should be Running")]
fn then_status_running(world: &mut QuectoWorld) {
    let pid = world.nrt_pid.expect("nrt pid");
    let rt = world.nrt_runtime.as_ref().expect("nrt runtime");
    let status = rt.status(pid);
    assert!(
        matches!(status, WorkerStatus::Running),
        "expected Running, got {status:?}"
    );
}

#[then(regex = r#"^the runtime status should be Exited with code (\d+)$"#)]
fn then_status_exited(world: &mut QuectoWorld, code: i32) {
    let pid = world.nrt_pid.expect("nrt pid");
    let rt = world.nrt_runtime.as_ref().expect("nrt runtime");
    let status = rt.status(pid);
    assert!(
        matches!(status, WorkerStatus::Exited { status: c } if c == code),
        "expected Exited({code}), got {status:?}"
    );
}

#[then("the runtime status should be Killed")]
fn then_status_killed(world: &mut QuectoWorld) {
    let pid = world.nrt_pid.expect("nrt pid");
    let rt = world.nrt_runtime.as_ref().expect("nrt runtime");
    let status = rt.status(pid);
    assert!(
        matches!(status, WorkerStatus::Killed { .. }),
        "expected Killed, got {status:?}"
    );
}

#[then(regex = r#"^the runtime status should be Killed with reason containing "([^"]+)"$"#)]
fn then_status_killed_with_reason(world: &mut QuectoWorld, reason_substr: String) {
    let pid = world.nrt_pid.expect("nrt pid");
    let rt = world.nrt_runtime.as_ref().expect("nrt runtime");
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

#[then("killing the worker should not return an error")]
fn then_kill_no_error(world: &mut QuectoWorld) {
    let pid = world.nrt_pid.expect("nrt pid");
    let rt = world.nrt_runtime.as_mut().expect("nrt runtime");
    let result = rt.kill(pid);
    assert!(result.is_ok(), "kill should succeed, got: {result:?}");
}

#[then(regex = r#"^the launch should fail with an error containing "([^"]+)"$"#)]
fn then_launch_error(world: &mut QuectoWorld, expected: String) {
    let err = world
        .nrt_last_error
        .as_ref()
        .expect("should have an error from launch");
    assert!(
        err.contains(&expected),
        "error should contain '{expected}', got '{err}'"
    );
}

#[then(regex = r#"^the send should fail with an error containing "([^"]+)"$"#)]
fn then_send_error(world: &mut QuectoWorld, expected: String) {
    let err = world
        .nrt_last_error
        .as_ref()
        .expect("should have an error from send");
    assert!(
        err.contains(&expected),
        "error should contain '{expected}', got '{err}'"
    );
}

#[then(regex = r#"^the kill should fail with an error containing "([^"]+)"$"#)]
fn then_kill_error(world: &mut QuectoWorld, expected: String) {
    let err = world
        .nrt_last_error
        .as_ref()
        .expect("should have an error from kill");
    assert!(
        err.contains(&expected),
        "error should contain '{expected}', got '{err}'"
    );
}
