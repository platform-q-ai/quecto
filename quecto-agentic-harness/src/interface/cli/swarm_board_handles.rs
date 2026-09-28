//! The coordination board's handles builder (#2270): composition's
//! `build_swarm_board_handles`, injected through [`super::CliComposition`]
//! and carried on [`super::CliContext`]. The interface holds and passes on
//! handles only; it never constructs a board use case or adapter. S13
//! threads it into `SwarmContext` and `HostedStore`.

use crate::application::swarm::dto::BoardLocation;
use crate::infrastructure::tools::swarm_board_dispatch::SwarmBoardHandles;

/// Builds the board handles over one board file.
pub type SwarmBoardHandlesBuilder = fn(BoardLocation) -> SwarmBoardHandles;
