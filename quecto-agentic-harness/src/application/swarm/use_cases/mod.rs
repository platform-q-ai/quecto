//! The coordination board's use cases (epic #2265), one per file. Each is
//! constructed only by `composition::swarm`; the dispatcher
//! (`infrastructure::tools::swarm_board_dispatch`) holds injected handles.

mod activate_member;
mod admit_member;
mod bootstrap_run;
mod claim_task;
mod create_run;
mod create_task;
mod join_run;
mod read_run_snapshot;
mod read_run_status;
mod read_task;
mod record_member_launch;
mod register_member_socket;
mod release_task;
mod release_unlaunched_member;
mod set_task_dependencies;

pub use activate_member::ActivateMember;
pub use admit_member::AdmitMember;
pub use bootstrap_run::BootstrapRun;
pub use claim_task::ClaimTask;
pub use create_run::CreateRun;
pub use create_task::CreateTask;
pub use join_run::JoinRun;
pub use read_run_snapshot::ReadRunSnapshot;
pub use read_run_status::ReadRunStatus;
pub use read_task::ReadTask;
pub use record_member_launch::RecordMemberLaunch;
pub use register_member_socket::RegisterMemberSocket;
pub use release_task::ReleaseTask;
pub use release_unlaunched_member::ReleaseUnlaunchedMember;
pub use set_task_dependencies::SetTaskDependencies;
