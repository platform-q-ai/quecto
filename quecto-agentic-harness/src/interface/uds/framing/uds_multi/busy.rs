use std::sync::atomic::Ordering;
/// Shared "agent is mid-turn" flag (#828). Set by the dispatch loop for the
/// duration of `agent.process()` (via [`BusyGuard`]), read by the accept loop.
/// The connect-time conversation snapshot is pushed to a newly-connected client
/// ONLY when this is `true` — i.e. the agent is busy and cannot answer a
/// `get_messages` promptly via the (blocked) single dispatch loop. When idle the
/// dispatch loop answers `get_messages` itself in FIFO order, so no unsolicited
/// bytes are written and clients that don't ask see no protocol change.
pub(crate) type BusyFlag = std::sync::Arc<std::sync::atomic::AtomicBool>;

/// RAII guard: marks the agent busy for the duration of a turn and, on drop
/// (normal completion, early return, or panic), restores what the flag was
/// (#828): a turn inside a guarded span between turns (#2708) leaves the
/// span busy.
pub(crate) struct BusyGuard(BusyFlag, bool);

impl BusyGuard {
    pub(crate) fn new(flag: &BusyFlag) -> Self {
        let was = flag.swap(true, Ordering::SeqCst);
        Self(flag.clone(), was)
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.store(self.1, Ordering::SeqCst);
    }
}
