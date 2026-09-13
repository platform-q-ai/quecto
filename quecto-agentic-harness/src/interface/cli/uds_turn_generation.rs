//! `TurnControl`'s flags (abort / steer / shutting-down), its defaults, and
//! the swarm control generation tracking shared by the reader and the
//! dispatch loop (#1721). Child module of `uds_cancel`.
use super::TurnControl;
use std::sync::Arc;

impl Default for TurnControl {
    fn default() -> Self {
        Self {
            pending_swarm_wake: std::sync::Mutex::new(None),
            deferred_swarm_wake: std::sync::atomic::AtomicU64::new(0),
            swarm_control: None,
            abort_requested: std::sync::atomic::AtomicBool::new(false),
            pending_steers: std::sync::atomic::AtomicUsize::new(0),
            control_generation: std::sync::atomic::AtomicU64::new(u64::MAX),
            shutting_down: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl TurnControl {
    /// Record a control generation seen on a receipt or wake; never regresses.
    pub fn observe_control_generation(&self, generation: u64) {
        self.control_generation
            .fetch_update(
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
                |current| match current {
                    u64::MAX => Some(generation),
                    seen if generation > seen => Some(generation),
                    _ => None,
                },
            )
            .ok();
    }

    /// The latest control generation seen, if any.
    pub fn control_generation(&self) -> Option<u64> {
        match self
            .control_generation
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            u64::MAX => None,
            seen => Some(seen),
        }
    }

    pub fn with_swarm_control(
        control: Option<Arc<dyn crate::domain::swarm::SwarmRunControl>>,
    ) -> Self {
        Self {
            swarm_control: control,
            ..Self::default()
        }
    }
}

impl TurnControl {
    /// Shutdown executor: no turn may start from here on (#1936).
    pub fn mark_shutting_down(&self) {
        self.shutting_down
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Whether a shutdown is executing and turn admission is closed.
    pub fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Reader: record a full-stop abort ahead of dispatch (#895).
    pub fn mark_abort(&self) {
        self.abort_requested
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Reader: record an explicit steer ahead of dispatch (#896).
    pub fn mark_steer(&self) {
        self.pending_steers
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Idle drain: consume the abort flag (true once, then cleared).
    pub fn take_abort(&self) -> bool {
        self.abort_requested
            .swap(false, std::sync::atomic::Ordering::SeqCst)
    }

    /// Whether a full-stop abort is queued but not yet consumed.
    pub fn is_abort_pending(&self) -> bool {
        self.abort_requested
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Whether a steer is queued but not yet handled.
    pub fn is_steer_pending(&self) -> bool {
        self.pending_steers
            .load(std::sync::atomic::Ordering::SeqCst)
            > 0
    }

    /// One admitted steering command has reached its handler. Other queued
    /// steering commands keep priority over buffered follow-ups.
    pub fn consume_steer(&self) {
        let _ = self.pending_steers.fetch_update(
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
            |pending| pending.checked_sub(1),
        );
    }

    /// Abort/reset releases all pending steering intent.
    pub fn clear_steer(&self) {
        self.pending_steers
            .store(0, std::sync::atomic::Ordering::SeqCst);
    }

    /// Clear both flags (abort handler / full reset).
    pub fn clear(&self) {
        self.abort_requested
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.clear_steer();
    }
}
