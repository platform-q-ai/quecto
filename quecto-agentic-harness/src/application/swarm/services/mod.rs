//! The swarm capability's internal helpers (#2367): what its board use
//! cases share, each a port of the Python board's private helpers, used
//! only by the capability's own use cases (never a use case, never a port).

pub(super) mod board_completion;
pub(super) mod board_control;
pub(super) mod board_loss;
pub(super) mod board_membership;
pub(super) mod board_messages;
pub(super) mod board_operation;
pub(super) mod board_read_models;
pub(super) mod board_recovery;
pub(super) mod board_tasks;
pub(super) mod board_wakes;
