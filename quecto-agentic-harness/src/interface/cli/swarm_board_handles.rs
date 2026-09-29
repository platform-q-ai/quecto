//! The coordination board's builders (#2270, #2278): composition's
//! `build_swarm_board_handles`, `board_wire` (#2279) and `board_op_log`,
//! injected through
//! [`super::CliComposition`] and carried on [`super::CliContext`] into the
//! agent run, which gives every `SwarmContext` one shared `SwarmBoard`.
//! The interface holds and passes on handles only; it never constructs a
//! board use case or adapter.

pub use crate::infrastructure::tools::swarm_board_dispatch::BoardWire;
pub use crate::infrastructure::tools::swarm_bridge::{
    SwarmBoardHandlesBuilder, SwarmBoardOpLogBuilder,
};
