/// Which pane currently has keyboard focus (#802). The editor (`Input`) is the
/// default; `Tab` toggles to the side `Panel` when sub-agents are listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    Input,
    Panel,
}

/// Width of the persistent left sub-agent panel (#800); room for names + a bar.
pub(crate) const SUBAGENT_PANEL_WIDTH: usize = 34;

/// Maximum retained sub-agent sessions before the oldest non-active is evicted.
pub(crate) const MAX_RETAINED_SESSIONS: usize = 16;
