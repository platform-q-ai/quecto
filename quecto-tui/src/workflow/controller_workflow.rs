#[derive(Default)]
pub(super) struct WorkflowFlow {
    /// Mirror of core workflow auto-continue state, toggled through UDS.
    pub(super) auto_continue: bool,
    /// Mirror of core workflow completion-nudge state, toggled through UDS.
    pub(super) completion_nudge: bool,
}
