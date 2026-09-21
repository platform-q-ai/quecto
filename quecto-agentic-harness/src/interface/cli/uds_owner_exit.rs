//! The owner's exit announcement at the dispatch boundary (#2070).
use super::DispatchCtx;
use crate::domain::session::SubagentRestoreReason;

/// The announcement as the dispatch context holds it.
pub type OwnerExit =
    std::sync::Arc<dyn crate::application::subagents::ports::OwnerExitAnnouncement>;

/// The owning TUI says it is exiting and will stop this harness (#2070):
/// the shutdown its signal then admits is the owner's word. Announced
/// before the save, so a save that fails changes nothing.
pub(super) fn announce_owner_exit(ctx: &DispatchCtx<'_>, reason: SubagentRestoreReason) {
    if reason == SubagentRestoreReason::OrdinaryTuiExitStopped
        && let Some(owner_exit) = &ctx.owner_exit
    {
        owner_exit.announce();
    }
}
