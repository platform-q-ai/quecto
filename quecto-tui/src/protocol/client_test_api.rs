//! Test-only constructors and probes for [`Client`], kept out of the main
//! module so the wire client stays within the module size gate.

use super::*;

impl Client {
    /// Try to receive an event without blocking (tests only).
    #[cfg(test)]
    pub fn try_recv(&mut self) -> Option<Event> {
        self.event_rx.try_recv().ok()
    }

    pub fn disconnected_for_tests() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<String>(1);
        drop(cmd_rx);
        let (_event_tx, event_rx) = mpsc::channel::<Event>(1);
        Self {
            cmd_tx,
            event_rx,
            dropped_oversized: Default::default(),
            speaks_frames: true,
        }
    }
}
