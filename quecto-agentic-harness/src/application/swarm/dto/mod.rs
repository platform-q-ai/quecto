//! Request, response and row types of the swarm board use cases (#2270).
//! The board ports name these beside the domain records; the dispatcher
//! (`infrastructure::tools::swarm_board_dispatch`) builds the requests from
//! a member's JSON arguments and renders the responses in Python's shape.

pub mod location;
pub mod measure;
pub mod membership;
pub mod run;
pub mod tasks;

pub use location::BoardLocation;
pub use measure::{CallMeasure, RunRoles};
pub use membership::{
    ActivateMemberRequest, AdmissionDecision, AdmitMemberRequest, AdmittedMember, JoinRunRequest,
    Joined, LaunchIdentity, RecordMemberLaunchRequest, RegisterMemberSocketRequest,
    ReleaseUnlaunchedMemberRequest,
};
pub use run::{
    BootstrapRunRequest, Bootstrapped, CreateBranch, CreateRunRequest, CreatedRun,
    MemberClaimCounts, MemberRow, NewMember, NewRun, RunContract, RunOwnerRow, RunSnapshotView,
    RunStatusRow, RunStatusView,
};
pub use tasks::{
    ClaimTaskRequest, CreateTaskRequest, CreatedTask, NewTask, ReadTaskRequest, ReleaseTaskRequest,
    SetTaskDependenciesRequest, TaskRow, TaskUpdate,
};
