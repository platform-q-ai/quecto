use super::*;

#[tokio::test]
async fn armed_child_still_runs_when_the_parent_lives() {
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg("exit 7");
    cmd.stdin(std::process::Stdio::null());
    arm(&mut cmd, std::process::id());
    let status = cmd.status().await.unwrap();
    assert_eq!(status.code(), Some(7));
}

#[tokio::test]
async fn a_wrong_expected_parent_makes_the_child_exit_before_exec() {
    // Simulates the race: the parent that forked is not the expected one.
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg("exit 0");
    cmd.stdin(std::process::Stdio::null());
    // pid 1 is never the test process.
    arm(&mut cmd, 1);
    let status = cmd.status().await.unwrap();
    #[cfg(target_os = "linux")]
    assert_eq!(status.code(), Some(1));
    #[cfg(not(target_os = "linux"))]
    assert_eq!(status.code(), Some(0));
}

#[tokio::test]
async fn arming_is_inert_for_a_zero_parent() {
    let mut cmd = tokio::process::Command::new("true");
    arm(&mut cmd, 0);
    assert!(cmd.status().await.unwrap().success());
}
