use super::*;
use std::path::Path;

/// Runs `attempt` (a stub run through a runner, with the deadline it is
/// given) with a deadline of one, then two, then four seconds, until
/// `ready` (what the stub leaves once its trap is set) is seen before that
/// deadline passed: only then can no SIGTERM have reached the stub before
/// its trap, however loaded the machine. `fresh` are removed before each
/// attempt. Answers that attempt's result and what `ready` saw.
pub(crate) fn ready_before_its_deadline<T: Send, R>(
    fresh: &[&Path],
    ready: impl Fn() -> Option<R>,
    attempt: impl Fn(Duration) -> T + Sync,
) -> (T, R) {
    for deadline in [1, 2, 4].map(Duration::from_secs) {
        for file in fresh {
            let _ = std::fs::remove_file(file);
        }
        let started = Instant::now();
        let (result, seen) = thread::scope(|scope| {
            let run = scope.spawn(|| attempt(deadline));
            let seen = loop {
                // Seen first, then the clock: a sighting before the deadline.
                let seen = ready();
                match (seen, started.elapsed() < deadline) {
                    (Some(seen), true) => break Some(seen),
                    (None, true) => thread::sleep(Duration::from_millis(5)),
                    (_, false) => break None,
                }
            };
            (run.join().unwrap(), seen)
        });
        if let Some(seen) = seen {
            return (result, seen);
        }
    }
    panic!("the stub never set its trap before its deadline");
}

/// A leader something else reaped (ECHILD: its pid, and so its group's,
/// may already be another's) has its group signalled with nothing: here
/// the group is still the one it led, and what it left running lives on.
#[cfg(target_os = "linux")]
#[test]
fn a_group_whose_leader_was_reaped_elsewhere_is_never_signalled() {
    use crate::infrastructure::processes::parent_death_signal::own_session_dying_with_us;
    use crate::infrastructure::processes::parent_death_signal::tests::Watched;
    let dir = tempfile::tempdir().unwrap();
    let pid = dir.path().join("pid");
    let mut command = std::process::Command::new("sh");
    command
        .args(["-c", &format!("sleep 30 & echo $! > '{}'", pid.display())])
        .stdin(std::process::Stdio::null());
    own_session_dying_with_us(&mut command);
    let mut leader = command.spawn().unwrap();
    let leader_pid = libc::pid_t::try_from(leader.id()).unwrap();
    let mut status = 0;
    // SAFETY: reaps our own child; status is writable.
    let reaped = unsafe { libc::waitpid(leader_pid, &mut status, 0) };
    assert_eq!(reaped, leader_pid);
    let sleeper = Watched::named_in(&pid).expect("the leader named what it left running");
    let ended = end_group(leader.id());
    thread::sleep(TERM_GRACE + Duration::from_millis(100));
    let lived = !sleeper.gone();
    sleeper.end();
    assert_eq!(ended, Ended::NotOurs);
    // Reaped elsewhere: there is nothing left for its Child to wait on.
    assert!(leader.wait().is_err());
    assert!(
        lived,
        "a group whose leader was reaped elsewhere was signalled"
    );
}

/// An exited, unreaped leader's group is ended at once, with no grace:
/// what it left running in the group is killed.
#[cfg(target_os = "linux")]
#[test]
fn an_exited_leaders_group_is_ended_without_a_grace() {
    use crate::infrastructure::processes::parent_death_signal::own_session_dying_with_us;
    use crate::infrastructure::processes::parent_death_signal::tests::Watched;
    let dir = tempfile::tempdir().unwrap();
    let pid = dir.path().join("pid");
    let mut command = std::process::Command::new("sh");
    command
        .args(["-c", &format!("sleep 30 & echo $! > '{}'", pid.display())])
        .stdin(std::process::Stdio::null());
    own_session_dying_with_us(&mut command);
    let mut leader = command.spawn().unwrap();
    while !exited_unreaped(leader.id()).unwrap() {
        thread::sleep(Duration::from_millis(5));
    }
    let left = Watched::named_in(&pid).expect("the leader named what it left running");
    let started = Instant::now();
    let ended = end_group(leader.id());
    let took = started.elapsed();
    let by = Instant::now() + Duration::from_secs(2);
    while left.alive() && Instant::now() < by {
        thread::sleep(Duration::from_millis(5));
    }
    let killed = left.gone();
    left.end();
    assert_eq!(ended, Ended::Signalled);
    assert!(took < TERM_GRACE, "{took:?}");
    assert!(
        killed,
        "what the exited leader left in its group outlived it"
    );
    assert!(leader.wait().unwrap().success());
}
