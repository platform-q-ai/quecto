//! The host's word on its clock (epic #2705), for the board server's
//! clock gate: on Linux, the kernel's NTP state.
use crate::application::project_board::ports::{ClockSync, TimeSync};

/// The kernel's word on its clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct KernelTimeSync;

impl TimeSync for KernelTimeSync {
    fn sync(&self) -> ClockSync {
        ClockSync::Unsynced("not read yet".into())
    }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn judged(_clock: libc::c_int, _status: libc::c_int) -> ClockSync {
    ClockSync::Unsynced("not judged yet".into())
}

#[cfg(test)]
#[path = "kernel_time_sync_tests.rs"]
mod tests;
