//! Request, response and row types of the swarm board use cases (#2270).
//! The board ports name these beside the domain records; the dispatcher
//! (`infrastructure::tools::swarm_board_dispatch`) builds the requests from
//! a member's JSON arguments and renders the responses in Python's shape.

pub mod completion;
pub mod control;
pub mod files;
pub mod location;
pub mod loss;
pub mod measure;
pub mod membership;
pub mod messages;
pub mod notifications;
pub mod reads;
pub mod run;
pub mod submissions;
pub mod tasks;
pub mod usage;

pub use completion::{
    AmendRunContractRequest, AmendedContract, CompleteRunRequest, CompletionState, EvidenceEntry,
    EvidenceTransition, NewEvidence, PriorEvidence, RecordEvidenceRequest, RevalidateTaskRequest,
    StoredContract,
};
pub use control::{
    ControlAnswer, ControlReceipt, ExtendRunDeadlineRequest, PauseRunRequest, RecentRequest,
    RunTransition, StopRunRequest, UsageReport, UsageRow,
};
pub use files::{
    FileRow, ListFileOwnersRequest, NewReservation, RecoverTaskRequest, Recovered,
    ReleaseFilesRequest, Reservation, ReserveFilesRequest, Revocation, RevokeTaskRequest, Revoked,
};
pub use location::BoardLocation;
pub use loss::{
    ConfirmMemberDeadRequest, CoordinatorLoss, DeathConfirmation, DeathConfirmed,
    LoseCoordinatorRequest, Quarantine, QuarantineMemberRequest, Quarantined, ScopeObservation,
};
pub use measure::{CallMeasure, RunRoles};
pub use membership::{
    ActivateMemberRequest, AdmissionDecision, AdmitMemberRequest, AdmittedMember, JoinRunRequest,
    Joined, LaunchIdentity, RecordMemberLaunchRequest, RegisterMemberSocketRequest,
    ReleaseUnlaunchedMemberRequest,
};
pub use messages::{
    MessageIdRequest, MessageRow, NewMessage, ReadInboxRequest, SendMessageRequest, SentMessage,
    Settled,
};
pub use notifications::{
    AcceptWakeRequest, ClaimNotificationsRequest, NotificationBatch, NotificationCursor,
    WakeAccepted,
};
pub use reads::{
    BootstrapMemberRequest, BootstrappedSummary, CountedTask, DictRow, EventPage, FullSummary,
    JoinedSummary, LatestActivity, ListTasksRequest, MessageTally, ReadRunEventsRequest,
    ReadRunSummaryRequest, RunSummary, RunTotalsView, SummaryCounts, SummaryScan, TaskPage,
};
pub use run::{
    BootstrapRunRequest, Bootstrapped, CreateBranch, CreateRunRequest, CreatedRun,
    MemberClaimCounts, MemberRow, MemberStatusRow, NewMember, NewRun, RunContract, RunOwnerRow,
    RunSnapshotView, RunStatusRow, RunStatusView,
};
pub use submissions::{
    BlockTaskRequest, SubmitTaskRequest, TaskChange, TaskTransition, UnblockTaskRequest,
    VerifyTaskRequest,
};
pub use tasks::{
    ClaimTaskRequest, CreateTaskRequest, CreatedTask, NewTask, ReadTaskRequest, ReleaseTaskRequest,
    SetTaskDependenciesRequest, TaskRow, TaskUpdate,
};
pub use usage::{
    BudgetChange, BudgetEffect, ConfigureUsageBudgetRequest, ConfiguredBudget, NewRequestUsage,
    RecordRequestUsageRequest, RecordedRequest, RequestAdmission, RequestDelivery,
    StoredRequestUsage,
};
