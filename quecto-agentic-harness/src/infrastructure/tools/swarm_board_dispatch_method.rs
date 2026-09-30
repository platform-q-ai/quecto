//! The board's method vocabulary (#2272 review nit): each method's name,
//! telemetry level and role, whether its answer proves the caller a
//! member, and its Python signature. Serving and argument binding stay in
//! the parent dispatch adapter.
//!
//! The dispatcher serves each method by its module. The task and claim
//! methods `task_create`, `dependencies`, `claim` and `release` (#2272) are
//! served by [`tasks`], with `task` and the test-only `task_raw` (`task`
//! without the owner liveness #2277 adds); `block`, `unblock`, `submit` and
//! `verify_task` by [`submissions`]; the run control methods `pause`,
//! `resume`, `stop`, `_resume_external`, `_close`, `_extend_deadline`,
//! `_control_status` and `usage_report` (#2273) by [`control`]; `complete`,
//! `revalidate_task`, `amend` and the criterion `evidence` success needs
//! (#2273) by [`completion`]; the usage methods `usage_budget`,
//! `_record_request` and `_request_admission` (#2274, its gate #2339) by
//! [`usage`]; and the file reservations `reserve`, `release_files` and
//! `file_owners`, with the coordinator's `recover` and `revoke` (#2275), by
//! [`reservations`]; and the durable messages `send`, `withdraw`, `inbox`
//! and `ack` (#2276) by [`messages`], the wake notifications
//! `_notifications` and `_accept_wake` by [`wakes`], the loss and death
//! records `_quarantine`, `_confirmed_dead` and `_lose_coordinator` (#2277)
//! by [`loss`], and the read models `summary`, `events` and `tasks`, with
//! the summaries `create`, `_join` and `_bootstrap` answer with (#2277), by
//! [`reads`].
use serde_json::Value;

use super::Level;
#[cfg(any(test, feature = "test-support"))]
use super::test_only;
use super::{
    completion, control, loss, members, messages, reads, reservations, submissions, tasks, usage,
    wakes,
};
use crate::domain::swarm::BoardRole;

/// The board methods this dispatcher serves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Method {
    Status,
    EventCursor,
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
    Reserve,
    ReleaseFiles,
    FileOwners,
    Recover,
    Revoke,
    Send,
    Withdraw,
    Inbox,
    Ack,
    Notifications,
    AcceptWake,
    Quarantine,
    ConfirmedDead,
    LoseCoordinator,
    Summary,
    Events,
    Task,
    Tasks,
    Create,
    Bootstrap,
    Join,
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
            "_event_cursor" => Some(Self::EventCursor),
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
            "reserve" => Some(Self::Reserve),
            "release_files" => Some(Self::ReleaseFiles),
            "file_owners" => Some(Self::FileOwners),
            "recover" => Some(Self::Recover),
            "revoke" => Some(Self::Revoke),
            "send" => Some(Self::Send),
            "withdraw" => Some(Self::Withdraw),
            "inbox" => Some(Self::Inbox),
            "ack" => Some(Self::Ack),
            "_notifications" => Some(Self::Notifications),
            "_accept_wake" => Some(Self::AcceptWake),
            "_quarantine" => Some(Self::Quarantine),
            "_confirmed_dead" => Some(Self::ConfirmedDead),
            "_lose_coordinator" => Some(Self::LoseCoordinator),
            "summary" => Some(Self::Summary),
            "events" => Some(Self::Events),
            "task" => Some(Self::Task),
            "tasks" => Some(Self::Tasks),
            "create" => Some(Self::Create),
            "_bootstrap" => Some(Self::Bootstrap),
            "_join" => Some(Self::Join),
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
            Self::EventCursor => "_event_cursor",
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
            Self::Reserve => "reserve",
            Self::ReleaseFiles => "release_files",
            Self::FileOwners => "file_owners",
            Self::Recover => "recover",
            Self::Revoke => "revoke",
            Self::Send => "send",
            Self::Withdraw => "withdraw",
            Self::Inbox => "inbox",
            Self::Ack => "ack",
            Self::Notifications => "_notifications",
            Self::AcceptWake => "_accept_wake",
            Self::Quarantine => "_quarantine",
            Self::ConfirmedDead => "_confirmed_dead",
            Self::LoseCoordinator => "_lose_coordinator",
            Self::Summary => "summary",
            Self::Events => "events",
            Self::Task => "task",
            Self::Tasks => "tasks",
            Self::Create => "create",
            Self::Bootstrap => "_bootstrap",
            Self::Join => "_join",
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
            | Self::EventCursor
            | Self::Snapshot
            | Self::ControlStatus
            | Self::UsageReport
            | Self::RequestAdmission
            | Self::FileOwners
            | Self::Inbox => Level::Read,
            // The wake claims run as Python's `read_only` operation and
            // after every board change; the cursor they move is recorded
            // as `cursor_moved`, not as a level.
            Self::Notifications | Self::AcceptWake => Level::Read,
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
            | Self::RecordRequest
            | Self::Reserve
            | Self::ReleaseFiles
            | Self::Recover
            | Self::Revoke
            | Self::Send
            | Self::Withdraw
            | Self::Ack => Level::Mutation,
            // The harness's loss and death records (#2277) write the
            // board when they decide to; each is rare, so INFO.
            Self::Quarantine | Self::ConfirmedDead | Self::LoseCoordinator => Level::Mutation,
            // The read models every member polls (the lifecycle watch
            // loop), #2277; the summaries `create`, `_join` and
            // `_bootstrap` answer with follow their writes.
            Self::Summary | Self::Events | Self::Task | Self::Tasks => Level::Read,
            Self::Create | Self::Bootstrap | Self::Join => Level::Mutation,
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
            // The structured ops' event cursor (#2279), read by the tool
            // around a member's call.
            Self::EventCursor => Some(BoardRole::Host),
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
            // The harness's wake hints, sent and accepted (#2276).
            Self::Notifications | Self::AcceptWake => Some(BoardRole::Host),
            // The harness's loss and death records (#2277).
            Self::Quarantine | Self::ConfirmedDead | Self::LoseCoordinator => Some(BoardRole::Host),
            // The harness's own join (#2277).
            Self::Bootstrap | Self::Join => Some(BoardRole::Host),
            Self::Summary | Self::Events | Self::Task | Self::Tasks | Self::Create => None,
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
            | Self::UsageBudget
            | Self::Reserve
            | Self::ReleaseFiles
            | Self::FileOwners
            | Self::Recover
            | Self::Revoke
            | Self::Send
            | Self::Withdraw
            | Self::Inbox
            | Self::Ack => None,
            // Test-only halves the differential harness drives as the host;
            // the member-facing `create` and `task` record the caller's own
            // role, and `_bootstrap` the host's (#2277).
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
            // Membership-free, like `_status` (#2279).
            Self::Status | Self::EventCursor => false,
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
            | Self::RequestAdmission
            | Self::Reserve
            | Self::ReleaseFiles
            | Self::FileOwners
            | Self::Recover
            | Self::Revoke
            | Self::Send
            | Self::Withdraw
            | Self::Inbox
            | Self::Ack
            | Self::Notifications
            | Self::AcceptWake
            | Self::Quarantine
            | Self::ConfirmedDead
            | Self::LoseCoordinator
            | Self::Summary
            | Self::Events
            | Self::Task
            | Self::Tasks => true,
            // `create` makes the caller the run's coordinator; the joins
            // admit and activate it, or find its own live row.
            Self::Create | Self::Bootstrap | Self::Join => true,
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
    /// the same name (`pause`'s `paused`) keeps its own level. It is
    /// lowered to [`Level::Read`] for a quarantine still inside its grace.
    pub(super) fn served_level(self, decision: &str) -> Level {
        match (self, decision) {
            (
                Self::UsageBudget | Self::RecordRequest | Self::RequestAdmission,
                "paused" | "warned",
            ) => Level::Mutation,
            // #2277 review N6: a reconcile retries a quarantine until its
            // grace ends, so one still inside the grace (which writes at
            // most the observer's first observation) is DEBUG; every other
            // decision keeps the method's INFO.
            (Self::Quarantine, "grace_pending") => Level::Read,
            _ => self.level(),
        }
    }

    /// The Python signature, `self` left out.
    pub(super) fn parameters(self) -> &'static [Parameter] {
        match self {
            Self::Status
            | Self::EventCursor
            | Self::Snapshot
            | Self::Resume
            | Self::ResumeExternal
            | Self::Close
            | Self::ControlStatus
            | Self::UsageReport
            | Self::LoseCoordinator => &[],
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
            Self::RequestAdmission => &usage::REQUEST_ADMISSION,
            Self::Reserve => &reservations::RESERVE,
            Self::ReleaseFiles => &reservations::RELEASE_FILES,
            Self::FileOwners => &reservations::FILE_OWNERS,
            Self::Recover => &reservations::RECOVER,
            Self::Revoke => &reservations::REVOKE,
            Self::Send => &messages::SEND,
            Self::Withdraw | Self::Ack => &messages::MESSAGE_ID,
            Self::Inbox => &messages::INBOX,
            Self::Notifications => &wakes::NOTIFICATIONS,
            Self::AcceptWake => &wakes::ACCEPT_WAKE,
            Self::Quarantine => &loss::QUARANTINE,
            Self::ConfirmedDead => &loss::CONFIRMED_DEAD,
            Self::Summary => &reads::SUMMARY,
            Self::Events => &reads::EVENTS,
            Self::Task => &tasks::CLAIM,
            Self::Tasks => &reads::TASKS,
            Self::Create => &reads::CREATE,
            Self::Bootstrap => &reads::BOOTSTRAP,
            Self::Join => &reads::JOIN,
            #[cfg(any(test, feature = "test-support"))]
            Self::CreateRun => &reads::CREATE,
            #[cfg(any(test, feature = "test-support"))]
            Self::BootstrapRun => &test_only::BOOTSTRAP,
            #[cfg(any(test, feature = "test-support"))]
            Self::BootstrapJoin => &reads::BOOTSTRAP,
            #[cfg(any(test, feature = "test-support"))]
            Self::TaskRaw => &tasks::CLAIM,
        }
    }
}
