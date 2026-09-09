//! Swarm control generation tracking shared by the reader and the dispatch
//! loop (#1721), plus the control's defaults. Child module of `uds_cancel`.
use super::TurnControl;
use std::sync::Arc;

impl Default for TurnControl {
    fn default() -> Self {
        Self {
            pending_swarm_wake: std::sync::Mutex::new(None),
            swarm_control: None,
            abort_requested: std::sync::atomic::AtomicBool::new(false),
            pending_steers: std::sync::atomic::AtomicUsize::new(0),
            control_generation: std::sync::atomic::AtomicU64::new(u64::MAX),
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
