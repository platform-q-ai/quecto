//! The coordination board's handles builder (#2270): composition's
//! `build_swarm_board_handles`, injected through [`super::CliComposition`].
//! The interface holds and passes on handles only; it never constructs a
//! board use case or adapter. Nothing reads it yet: S13 (#2278) carries it
//! on [`super::CliContext`] and threads it into `SwarmContext` and
//! `HostedStore`.

use std::sync::Arc;

use crate::application::swarm::dto::BoardLocation;
use crate::application::swarm::ports::BoardOpLog;
use crate::infrastructure::tools::swarm_board_dispatch::SwarmBoardHandles;

/// Builds the board handles over one board file, recording each call in
/// the event log given (composition's `board_op_log`, only while the event
/// log is on, #2303).
pub type SwarmBoardHandlesBuilder =
    fn(BoardLocation, Option<Arc<dyn BoardOpLog>>) -> SwarmBoardHandles;
