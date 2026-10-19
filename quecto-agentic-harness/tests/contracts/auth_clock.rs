//! Contract for the auth capability's `AuthClock` (#1839): the system
//! clock reads this machine's Unix time, to the second.
use quecto::application::auth::ports::AuthClock;
use quecto::infrastructure::time::auth_clock::SystemAuthClock;

fn system_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

#[test]
fn the_system_clock_reads_unix_seconds_now() {
    let clock: &dyn AuthClock = &SystemAuthClock;
    let before = system_now();
    let read = clock.now_unix_secs();
    let after = system_now();
    assert!(
        before <= read && read <= after,
        "{before} <= {read} <= {after}"
    );
}
