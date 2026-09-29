//! The coordination board's use cases (epic #2265), one per file. Each is
//! constructed only by `composition::swarm`; the dispatcher
//! (`infrastructure::tools::swarm_board_dispatch`) holds injected handles.
//!
//! While the event log is on (#2303), a call is served over its own
//! metered repository: the dispatcher asks the composed handle for the
//! same use case [`OverRepository::over`] that repository, which keeps
//! every other port composition bound and constructs nothing else, so the
//! graph is composed once, not per call (round-3 review M1).
use std::sync::Arc;

use crate::application::swarm::ports::BoardRepository;

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

/// A board use case served over another repository (#2303): the same use
/// case, with every other port as composition bound it. A seam between
/// the board's use cases and their dispatcher, not a port: nothing outside
/// the crate implements it.
pub(crate) trait OverRepository: Sized {
    /// This use case over `repository`.
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self;
}
