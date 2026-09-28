//! #2237: the exit wait is awaited inside the bash tool's boxed `Send`
//! future, so it must be `Send` on every Unix, macOS included, where
//! `libc::siginfo_t` holds a raw pointer and is not `Send`. Checked at
//! compile time on every target; the closure is never called.
#![cfg(unix)]
use super::*;

fn assert_send<T: Send>(value: T) -> T {
    value
}

#[test]
fn the_exit_wait_future_is_send_on_every_unix() {
    let _check = |child: &mut tokio::process::Child| {
        drop(assert_send(exited(child)));
    };
}

/// The longest any exit wait here may take: every child exits well within
/// it, so a wait that never ends fails the test instead of hanging it.
const BOUND: std::time::Duration = std::time::Duration::from_secs(10);

/// `exited`, failing the test when it has not returned within [`BOUND`].
async fn bounded_exit(child: &mut tokio::process::Child) -> std::io::Result<()> {
    tokio::time::timeout(BOUND, exited(child))
        .await
        .expect("the exit wait returns within its bound")
}

#[tokio::test]
async fn an_exited_child_is_observed_and_left_unreaped() {
    let mut child = tokio::process::Command::new("sh")
        .args(["-c", "exit 3"])
        .spawn()
        .expect("sh spawns");
    bounded_exit(&mut child).await.expect("exit is observed");
    assert!(child.id().is_some(), "observing exit never reaps the child");
    let status = child.wait().await.expect("the owner reaps it");
    assert_eq!(status.code(), Some(3));
}

#[tokio::test]
async fn a_reaped_child_is_an_error_not_a_wait() {
    let mut child = tokio::process::Command::new("sh")
        .args(["-c", "exit 0"])
        .spawn()
        .expect("sh spawns");
    child.wait().await.expect("reaped");
    let error = bounded_exit(&mut child)
        .await
        .expect_err("nothing left to observe");
    assert!(error.to_string().contains("already reaped"), "{error}");
}

/// A pid that is not our child is a wait error, reported, never read as
/// "exited" (pid 1 is never this test's child; `WNOWAIT` reaps nothing).
#[test]
fn a_pid_that_is_not_our_child_is_an_error() {
    let error = has_exited(1).expect_err("pid 1 is not our child");
    assert_eq!(error.raw_os_error(), Some(libc::ECHILD), "{error}");
}

/// The wait returns only once the child has exited: a child still running
/// is waited for, never reported as exited.
#[tokio::test]
async fn a_running_child_is_waited_for_until_it_exits() {
    let mut child = tokio::process::Command::new("sh")
        .args(["-c", "sleep 0.3; exit 5"])
        .spawn()
        .expect("sh spawns");
    let pid = child.id().expect("running");
    assert!(
        !has_exited(pid).expect("a running child polls"),
        "not yet exited"
    );
    bounded_exit(&mut child).await.expect("exit is observed");
    assert!(has_exited(pid).expect("an exited child polls"), "exited");
    let status = child.wait().await.expect("the owner reaps it");
    assert_eq!(status.code(), Some(5));
}

/// How one `waitid` poll reads: a zero `si_pid` is "not yet", any other the
/// child's exit; an interrupted call is "not yet" too, polled again; any
/// other failure is reported.
#[test]
fn a_poll_reads_as_exited_not_yet_or_an_error() {
    assert!(!poll_outcome(Ok(0)).unwrap());
    assert!(poll_outcome(Ok(4242)).unwrap());
    assert!(!poll_outcome(Err(std::io::Error::from_raw_os_error(libc::EINTR))).unwrap());
    let error = poll_outcome(Err(std::io::Error::from_raw_os_error(libc::ECHILD)))
        .expect_err("a failed wait is reported");
    assert_eq!(error.raw_os_error(), Some(libc::ECHILD));
}

/// The poll observes an exit (`WEXITED`) without blocking (`WNOHANG`) and
/// without reaping (`WNOWAIT`): all three, nothing else.
#[test]
fn the_poll_observes_exit_without_blocking_or_reaping() {
    assert_eq!(
        OBSERVE_EXIT_UNREAPED,
        libc::WEXITED | libc::WNOHANG | libc::WNOWAIT
    );
    for flag in [libc::WEXITED, libc::WNOHANG, libc::WNOWAIT] {
        assert_ne!(OBSERVE_EXIT_UNREAPED & flag, 0, "{flag:#x}");
    }
}
