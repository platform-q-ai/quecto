use super::*;

fn sh(script: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c").arg(script);
    cmd.stdin(std::process::Stdio::null());
    cmd
}

#[test]
fn an_armed_child_runs_while_its_parent_lives() {
    let mut cmd = sh("exit 7");
    arm(&mut cmd, std::process::id());
    assert_eq!(cmd.status().unwrap().code(), Some(7));
}

#[test]
fn a_wrong_expected_parent_makes_the_child_exit_before_exec() {
    let mut cmd = sh("exit 0");
    // pid 1 is never the test process: the race where the parent that
    // forked is already gone.
    arm(&mut cmd, 1);
    let status = cmd.status().unwrap();
    #[cfg(target_os = "linux")]
    assert_eq!(status.code(), Some(1));
    #[cfg(not(target_os = "linux"))]
    assert_eq!(status.code(), Some(0));
}

#[test]
fn arming_is_inert_for_a_zero_parent() {
    let mut cmd = std::process::Command::new("true");
    arm(&mut cmd, 0);
    assert!(cmd.status().unwrap().success());
}

/// The property (#2053): once the launcher is gone, the kernel ends the
/// armed child with SIGTERM. The launcher here is a thread — the signal
/// fires on the death of the thread that forked — and the child is a bare
/// `sleep`, which has no handler, so the status names the signal itself.
#[cfg(target_os = "linux")]
#[test]
fn the_armed_child_receives_sigterm_when_its_launcher_dies() {
    use std::os::unix::process::ExitStatusExt;
    let mut child = std::thread::spawn(|| {
        let mut cmd = std::process::Command::new("sleep");
        cmd.arg("30");
        cmd.stdin(std::process::Stdio::null());
        arm(&mut cmd, std::process::id());
        cmd.spawn().unwrap()
    })
    .join()
    .unwrap();
    let status = child.wait().unwrap();
    assert_eq!(
        status.signal(),
        Some(libc::SIGTERM),
        "the kernel ended the child once its launcher thread ended: {status:?}"
    );
}
