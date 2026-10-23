//! Contract for the `LaunchedAuthority` port (#2615): a launched broker
//! reads `Running` while it serves, and a second launch over a served
//! directory exits Busy and reads `LostRace`, the winner serving on. Over
//! the real `quecto` binary, tethered to the test thread (#2631).
#![cfg(target_os = "linux")]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use quecto::application::admission::ports::{AuthorityLauncher, LaunchState, LaunchedAuthority};
use quecto::infrastructure::admission::DetachedAuthorityLauncher;

use super::authority_launcher::launch_for;

const LIMIT: Duration = Duration::from_secs(20);

fn settled(broker: &mut Box<dyn LaunchedAuthority>) -> LaunchState {
    let deadline = Instant::now() + LIMIT;
    loop {
        match broker.state() {
            LaunchState::Running => {
                assert!(Instant::now() < deadline, "the broker never exited");
                std::thread::sleep(Duration::from_millis(20));
            }
            settled => return settled,
        }
    }
}

#[test]
fn a_serving_broker_runs_and_a_second_launch_loses_the_lock() {
    let temp = tempfile::tempdir().unwrap();
    let launch = launch_for(temp.path());
    let launcher = DetachedAuthorityLauncher::dying_with_launching_thread(PathBuf::from(env!(
        "CARGO_BIN_EXE_quecto"
    )));
    let mut first = launcher.launch(&launch).unwrap();
    let deadline = Instant::now() + LIMIT;
    let mut attempt = 0;
    while launcher.probe(&launch.directory).is_err() {
        assert_eq!(first.state(), LaunchState::Running);
        assert!(
            Instant::now() < deadline,
            "the launched broker never answered"
        );
        launcher.pause(attempt.min(3));
        attempt += 1;
    }
    let mut second = launcher.launch(&launch).unwrap();
    assert_eq!(settled(&mut second), LaunchState::LostRace);
    assert_eq!(first.state(), LaunchState::Running, "the winner serves on");
    assert_eq!(launcher.probe(&launch.directory), Ok(()));
}
