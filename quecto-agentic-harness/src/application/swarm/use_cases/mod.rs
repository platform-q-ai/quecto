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

mod accept_wake;
mod acknowledge_message;
mod activate_member;
mod admit_member;
mod amend_run_contract;
mod block_task;
mod bootstrap_run;
mod claim_notifications;
mod claim_task;
mod close_run;
mod complete_run;
mod configure_usage_budget;
mod confirm_member_dead;
mod create_run;
mod create_task;
mod extend_run_deadline;
mod join_run;
mod list_file_owners;
mod lose_coordinator;
mod pause_run;
mod quarantine_member;
mod read_control_status;
mod read_inbox;
mod read_request_admission;
mod read_run_snapshot;
mod read_run_status;
mod read_task;
mod read_usage_report;
mod record_evidence;
mod record_member_launch;
mod record_request_usage;
mod recover_task;
mod register_member_socket;
mod release_files;
mod release_task;
mod release_unlaunched_member;
mod reserve_files;
mod resume_run;
mod resume_run_externally;
mod revalidate_task;
mod revoke_task;
mod send_message;
mod set_task_dependencies;
mod stop_run;
mod submit_task;
mod unblock_task;
mod verify_task;
mod withdraw_message;

pub use accept_wake::AcceptWake;
pub use acknowledge_message::AcknowledgeMessage;
pub use activate_member::ActivateMember;
pub use admit_member::AdmitMember;
pub use amend_run_contract::AmendRunContract;
pub use block_task::BlockTask;
pub use bootstrap_run::BootstrapRun;
pub use claim_notifications::ClaimNotifications;
pub use claim_task::ClaimTask;
pub use close_run::CloseRun;
pub use complete_run::CompleteRun;
pub use configure_usage_budget::ConfigureUsageBudget;
pub use confirm_member_dead::ConfirmMemberDead;
pub use create_run::CreateRun;
pub use create_task::CreateTask;
pub use extend_run_deadline::ExtendRunDeadline;
pub use join_run::JoinRun;
pub use list_file_owners::ListFileOwners;
pub use lose_coordinator::LoseCoordinator;
pub use pause_run::PauseRun;
pub use quarantine_member::QuarantineMember;
pub use read_control_status::ReadControlStatus;
pub use read_inbox::ReadInbox;
pub use read_request_admission::ReadRequestAdmission;
pub use read_run_snapshot::ReadRunSnapshot;
pub use read_run_status::ReadRunStatus;
pub use read_task::ReadTask;
pub use read_usage_report::ReadUsageReport;
pub use record_evidence::RecordEvidence;
pub use record_member_launch::RecordMemberLaunch;
pub use record_request_usage::RecordRequestUsage;
pub use recover_task::RecoverTask;
pub use register_member_socket::RegisterMemberSocket;
pub use release_files::ReleaseFiles;
pub use release_task::ReleaseTask;
pub use release_unlaunched_member::ReleaseUnlaunchedMember;
pub use reserve_files::ReserveFiles;
pub use resume_run::ResumeRun;
pub use resume_run_externally::ResumeRunExternally;
pub use revalidate_task::RevalidateTask;
pub use revoke_task::RevokeTask;
pub use send_message::SendMessage;
pub use set_task_dependencies::SetTaskDependencies;
pub use stop_run::StopRun;
pub use submit_task::SubmitTask;
pub use unblock_task::UnblockTask;
pub use verify_task::VerifyTask;
pub use withdraw_message::WithdrawMessage;

/// A board use case served over another repository (#2303): the same use
/// case, with every other port as composition bound it. A seam between
/// the board's use cases and their dispatcher, not a port: nothing outside
/// the crate implements it.
pub(crate) trait OverRepository: Sized {
    /// This use case over `repository`.
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self;
}
