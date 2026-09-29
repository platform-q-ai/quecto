//! The board's method vocabulary (#2272 review nit): each method's name,
//! telemetry level and role, whether its answer proves the caller a
//! member, and its Python signature. Serving and argument binding stay in
//! the parent dispatch adapter.
use serde_json::Value;

use super::Level;
#[cfg(any(test, feature = "test-support"))]
use super::test_only;
use super::{completion, control, members, submissions, tasks, usage};
use crate::domain::swarm::BoardRole;

/// The board methods this dispatcher serves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Method {
    Status,
    Snapshot,
    Admit,
    Activate,
    RecordLaunch,
    ReleaseUnlaunched,
    Socket,
    TaskCreate,
    Dependencies,
    Claim,
    Release,
    Block,
    Unblock,
    Submit,
    VerifyTask,
    Pause,
    Resume,
    ResumeExternal,
    Close,
    ExtendDeadline,
    Stop,
    ControlStatus,
    UsageReport,
    Complete,
    RevalidateTask,
    Amend,
    Evidence,
    UsageBudget,
    RecordRequest,
    RequestAdmission,
    #[cfg(any(test, feature = "test-support"))]
    CreateRun,
    #[cfg(any(test, feature = "test-support"))]
    BootstrapRun,
    #[cfg(any(test, feature = "test-support"))]
    BootstrapJoin,
    #[cfg(any(test, feature = "test-support"))]
    TaskRaw,
}

/// One parameter of a Python signature: required, or with a default.
#[derive(Clone, Copy)]
pub(super) struct Parameter {
    pub(super) name: &'static str,
    pub(super) default: Option<fn() -> Value>,
}

pub(super) const fn required(name: &'static str) -> Parameter {
    Parameter {
        name,
        default: None,
    }
}

impl Method {
    pub(super) fn parse(name: &str) -> Option<Self> {
        match name {
            "_status" => Some(Self::Status),
            "_snapshot" => Some(Self::Snapshot),
            "_admit" => Some(Self::Admit),
            "_activate" => Some(Self::Activate),
            "_record_launch" => Some(Self::RecordLaunch),
            "_release_unlaunched" => Some(Self::ReleaseUnlaunched),
            "_socket" => Some(Self::Socket),
            "task_create" => Some(Self::TaskCreate),
            "dependencies" => Some(Self::Dependencies),
            "claim" => Some(Self::Claim),
            "release" => Some(Self::Release),
            "block" => Some(Self::Block),
            "unblock" => Some(Self::Unblock),
            "submit" => Some(Self::Submit),
            "verify_task" => Some(Self::VerifyTask),
            "pause" => Some(Self::Pause),
            "resume" => Some(Self::Resume),
            "_resume_external" => Some(Self::ResumeExternal),
            "_close" => Some(Self::Close),
            "_extend_deadline" => Some(Self::ExtendDeadline),
            "stop" => Some(Self::Stop),
            "_control_status" => Some(Self::ControlStatus),
            "usage_report" => Some(Self::UsageReport),
            "complete" => Some(Self::Complete),
            "revalidate_task" => Some(Self::RevalidateTask),
            "amend" => Some(Self::Amend),
            "evidence" => Some(Self::Evidence),
            "usage_budget" => Some(Self::UsageBudget),
            "_record_request" => Some(Self::RecordRequest),
            "_request_admission" => Some(Self::RequestAdmission),
            #[cfg(any(test, feature = "test-support"))]
            "create_run" => Some(Self::CreateRun),
            #[cfg(any(test, feature = "test-support"))]
            "bootstrap_run" => Some(Self::BootstrapRun),
            #[cfg(any(test, feature = "test-support"))]
            "bootstrap_join" => Some(Self::BootstrapJoin),
            #[cfg(any(test, feature = "test-support"))]
            "task_raw" => Some(Self::TaskRaw),
            _ => None,
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Status => "_status",
            Self::Snapshot => "_snapshot",
            Self::Admit => "_admit",
            Self::Activate => "_activate",
            Self::RecordLaunch => "_record_launch",
            Self::ReleaseUnlaunched => "_release_unlaunched",
            Self::Socket => "_socket",
            Self::TaskCreate => "task_create",
            Self::Dependencies => "dependencies",
            Self::Claim => "claim",
            Self::Release => "release",
            Self::Block => "block",
            Self::Unblock => "unblock",
            Self::Submit => "submit",
            Self::VerifyTask => "verify_task",
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::ResumeExternal => "_resume_external",
            Self::Close => "_close",
            Self::ExtendDeadline => "_extend_deadline",
            Self::Stop => "stop",
            Self::ControlStatus => "_control_status",
            Self::UsageReport => "usage_report",
            Self::Complete => "complete",
            Self::RevalidateTask => "revalidate_task",
            Self::Amend => "amend",
            Self::Evidence => "evidence",
            Self::UsageBudget => "usage_budget",
            Self::RecordRequest => "_record_request",
            Self::RequestAdmission => "_request_admission",
            #[cfg(any(test, feature = "test-support"))]
            Self::CreateRun => "create_run",
            #[cfg(any(test, feature = "test-support"))]
            Self::BootstrapRun => "bootstrap_run",
            #[cfg(any(test, feature = "test-support"))]
            Self::BootstrapJoin => "bootstrap_join",
            #[cfg(any(test, feature = "test-support"))]
            Self::TaskRaw => "task_raw",
        }
    }

    /// [`Level::Read`] only for the methods listed as read-only.
    pub(super) fn level(self) -> Level {
        match self {
            // The admission read may pause the run by its budget; it runs
            // before every model request, so it records at DEBUG as the
            // read Python's `read_only` operation makes it, unless the
            // budget warned or paused (`served_level`).
            Self::Status
            | Self::Snapshot
            | Self::ControlStatus
            | Self::UsageReport
            | Self::RequestAdmission => Level::Read,
            Self::Admit
            | Self::Activate
            | Self::RecordLaunch
            | Self::ReleaseUnlaunched
            | Self::Socket
            | Self::TaskCreate
            | Self::Dependencies
            | Self::Claim
            | Self::Release
            | Self::Block
            | Self::Unblock
            | Self::Submit
            | Self::VerifyTask
            | Self::Pause
            | Self::Resume
            | Self::ResumeExternal
            | Self::Close
            | Self::ExtendDeadline
            | Self::Stop
            | Self::Complete
            | Self::RevalidateTask
            | Self::Amend
            | Self::Evidence
            | Self::UsageBudget
            | Self::RecordRequest => Level::Mutation,
            #[cfg(any(test, feature = "test-support"))]
            Self::CreateRun | Self::BootstrapRun | Self::BootstrapJoin => Level::Mutation,
            #[cfg(any(test, feature = "test-support"))]
            Self::TaskRaw => Level::Read,
        }
    }

    /// Who calls it (#2303): [`BoardRole::Host`] for the harness's own
    /// ops; `None` for a member-facing op, whose role is the caller's in
    /// the run, as the call's own measure read it
    /// ([`run_role`](crate::domain::swarm::telemetry::run_role)).
    pub(super) fn role(self) -> Option<BoardRole> {
        match self {
            Self::Status
            | Self::Snapshot
            | Self::Admit
            | Self::Activate
            | Self::RecordLaunch
            | Self::ReleaseUnlaunched
            | Self::Socket => Some(BoardRole::Host),
            // The supervisor's run control, outside the swarm (#2273).
            Self::ResumeExternal | Self::Close | Self::ExtendDeadline | Self::ControlStatus => {
                Some(BoardRole::Host)
            }
            // The harness's request ledger and inference admission read
            // (#2274).
            Self::RecordRequest | Self::RequestAdmission => Some(BoardRole::Host),
            Self::TaskCreate
            | Self::Dependencies
            | Self::Claim
            | Self::Release
            | Self::Block
            | Self::Unblock
            | Self::Submit
            | Self::VerifyTask
            | Self::Pause
            | Self::Resume
            | Self::Stop
            | Self::UsageReport
            | Self::Complete
            | Self::RevalidateTask
            | Self::Amend
            | Self::Evidence
            | Self::UsageBudget => None,
            // Test-only halves the differential harness drives as the host;
            // the member-facing `create`, `_bootstrap` and `task` S12 serves
            // record the caller's own role.
            #[cfg(any(test, feature = "test-support"))]
            Self::CreateRun | Self::BootstrapRun | Self::BootstrapJoin | Self::TaskRaw => {
                Some(BoardRole::Host)
            }
        }
    }

    /// Whether an answer proves the caller a member of the run (#2303
    /// round-4 review L1): the op checks membership, or makes the caller
    /// the run's coordinator. `_status` is membership-free, so any id can
    /// have it answered.
    pub(super) fn answers_members_only(self) -> bool {
        match self {
            Self::Status => false,
            // Through the operation gate, which refuses a caller that is
            // no member of the run (or whose death was confirmed).
            Self::Snapshot
            | Self::Admit
            | Self::Activate
            | Self::RecordLaunch
            | Self::ReleaseUnlaunched
            | Self::Socket
            | Self::TaskCreate
            | Self::Dependencies
            | Self::Claim
            | Self::Release
            | Self::Block
            | Self::Unblock
            | Self::Submit
            | Self::VerifyTask
            | Self::Pause
            | Self::ResumeExternal
            | Self::Close
            | Self::ExtendDeadline
            | Self::Stop
            | Self::ControlStatus
            | Self::UsageReport
            | Self::Complete
            | Self::RevalidateTask
            | Self::Amend
            | Self::Evidence
            | Self::UsageBudget
            | Self::RecordRequest
            | Self::RequestAdmission => true,
            // A member's own resume is refused before any gate (#2273), so
            // it never answers.
            Self::Resume => false,
            // `create_run` and `bootstrap_run` make the caller the run's
            // coordinator; `bootstrap_join` admits and activates it, or
            // finds its own live row; `task_raw` passes the gate.
            #[cfg(any(test, feature = "test-support"))]
            Self::CreateRun | Self::BootstrapRun | Self::BootstrapJoin | Self::TaskRaw => true,
        }
    }

    /// The level a served call records at: [`Method::level`], raised to
    /// [`Level::Mutation`] for a method the token budget applies in when
    /// the decision is the budget's own (`paused` ends the run as
    /// `budget-exhausted`, `warned` writes the warning), so a read that
    /// changes the run is visible at INFO. Only the usage methods decide
    /// `paused` or `warned` as the budget's; another method's decision of
    /// the same name (`pause`'s `paused`) keeps its own level.
    pub(super) fn served_level(self, decision: &str) -> Level {
        match (self, decision) {
            (
                Self::UsageBudget | Self::RecordRequest | Self::RequestAdmission,
                "paused" | "warned",
            ) => Level::Mutation,
            _ => self.level(),
        }
    }

    /// The Python signature, `self` left out.
    pub(super) fn parameters(self) -> &'static [Parameter] {
        match self {
            Self::Status
            | Self::Snapshot
            | Self::Resume
            | Self::ResumeExternal
            | Self::Close
            | Self::ControlStatus
            | Self::UsageReport
            | Self::RequestAdmission => &[],
            Self::Admit => &members::ADMIT,
            Self::Activate => &members::ACTIVATE,
            Self::RecordLaunch => &members::RECORD_LAUNCH,
            Self::ReleaseUnlaunched => &members::RELEASE_UNLAUNCHED,
            Self::Socket => &members::SOCKET,
            Self::TaskCreate => &tasks::TASK_CREATE,
            Self::Dependencies => &tasks::DEPENDENCIES,
            Self::Claim => &tasks::CLAIM,
            Self::Release => &tasks::RELEASE,
            Self::Block | Self::Unblock => &submissions::BLOCK,
            Self::Submit => &submissions::SUBMIT,
            Self::VerifyTask => &submissions::VERIFY_TASK,
            Self::Pause => &control::PAUSE,
            Self::ExtendDeadline => &control::EXTEND,
            Self::Stop => &control::STOP,
            Self::Complete => &completion::COMPLETE,
            Self::RevalidateTask => &completion::REVALIDATE_TASK,
            Self::Amend => &completion::AMEND,
            Self::Evidence => &completion::EVIDENCE,
            Self::UsageBudget => &usage::USAGE_BUDGET,
            Self::RecordRequest => &usage::RECORD_REQUEST,
            #[cfg(any(test, feature = "test-support"))]
            Self::CreateRun => &test_only::CREATE,
            #[cfg(any(test, feature = "test-support"))]
            Self::BootstrapRun => &test_only::BOOTSTRAP,
            #[cfg(any(test, feature = "test-support"))]
            Self::BootstrapJoin => &test_only::JOIN,
            #[cfg(any(test, feature = "test-support"))]
            Self::TaskRaw => &tasks::CLAIM,
        }
    }
}
