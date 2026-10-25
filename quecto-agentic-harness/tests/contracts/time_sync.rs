//! Contract for [`TimeSync`] (epic #2705), run against the kernel adapter
//! on the host running the tests: it answers every time without failing,
//! cheaply (it is asked on each reading of the board server's clock), and
//! an unsynchronised answer says why. Off Linux it never affirms
//! the clock synchronised.
use std::time::{Duration, Instant};

use quecto::application::project_board::ports::{ClockSync, TimeSync};
use quecto::infrastructure::time::kernel_time_sync::KernelTimeSync;

/// How many asks the cost check makes, and the time they must fit in.
const ASKS: u32 = 1_000;
const WITHIN: Duration = Duration::from_secs(2);

fn contract(sync: &dyn TimeSync) {
    let began = Instant::now();
    for _ in 0..ASKS {
        match sync.sync() {
            ClockSync::Synced => {}
            ClockSync::Unsynced(why) => assert!(!why.trim().is_empty(), "says why"),
        }
    }
    assert!(
        began.elapsed() < WITHIN,
        "{ASKS} asks took {:?}",
        began.elapsed()
    );
}

#[test]
fn the_kernel_adapter_keeps_the_time_sync_contract() {
    contract(&KernelTimeSync);
}

#[cfg(not(target_os = "linux"))]
#[test]
fn off_linux_the_clock_is_never_affirmed_synchronised() {
    assert!(matches!(KernelTimeSync.sync(), ClockSync::Unsynced(_)));
}
