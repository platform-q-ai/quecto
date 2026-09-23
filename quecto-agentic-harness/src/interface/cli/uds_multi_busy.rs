use std::sync::atomic::Ordering;
/// Shared "agent is mid-turn" flag (#828). Set by the dispatch loop for the
/// duration of `agent.process()` (via [`BusyGuard`]), read by the accept loop.
/// The connect-time conversation snapshot is pushed to a newly-connected client
/// ONLY when this is `true` — i.e. the agent is busy and cannot answer a
/// `get_messages` promptly via the (blocked) single dispatch loop. When idle the
/// dispatch loop answers `get_messages` itself in FIFO order, so no unsolicited
/// bytes are written and clients that don't ask see no protocol change.
pub(crate) type BusyFlag = std::sync::Arc<std::sync::atomic::AtomicBool>;

/// RAII guard: marks the agent busy for the duration of a turn and clears the
/// flag on drop (normal completion, early return, or panic) (#828).
pub(crate) struct BusyGuard(BusyFlag);

impl BusyGuard {
    pub(crate) fn new(flag: &BusyFlag) -> Self {
        flag.store(true, Ordering::SeqCst);
        Self(flag.clone())
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
