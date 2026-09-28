//! Request, response and row types of the swarm board use cases (#2270).
//! The board ports name these beside the domain records; the dispatcher
//! (`infrastructure::tools::swarm_board_dispatch`) builds the requests from
//! a member's JSON arguments and renders the responses in Python's shape.

pub mod location;
pub mod run;

pub use location::BoardLocation;
pub use run::{
    BootstrapRunRequest, Bootstrapped, CreateBranch, CreateRunRequest, CreatedRun,
    MemberClaimCounts, MemberRow, NewMember, NewRun, RunContract, RunOwnerRow, RunSnapshotView,
    RunStatusRow, RunStatusView,
};
