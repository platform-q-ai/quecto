//! The external-agent capability's own DTOs (#2285): what the projection
//! answers, and what its process ports take and answer (#2286). The
//! interface layer renders them on the wire.

pub mod audit;
pub mod background;
pub mod launch_spec;
pub mod message;
pub mod process;
pub mod report;
pub mod session;
pub mod step;
pub mod telemetry;
pub mod turn;

pub use self::audit::{
    ADMISSION_WARNING_CAPACITY, AUDIT_INPUT_PREVIEW_BYTES, GUARDRAIL_AUDIT_CAPACITY,
    GuardrailDenial,
};
pub use self::background::{BACKGROUND_JOB_CAPACITY, BackgroundJob, TERMINAL_TASK_STATUSES};
pub use self::launch_spec::{CredentialEnv, ExternalAgentLaunchSpec};
pub use self::message::{
    ExecutionState, MessageRole, ProjectedMessage, ProjectedToolCall, TOOL_RESULT_CONTENT_BYTES,
    TRUNCATION_MARKER,
};
pub use self::process::{
    AgentClockInstant, EXTERNAL_AGENT_STDERR_TAIL_BYTES, ExternalAgentExit,
    ExternalAgentInputError, ExternalAgentLaunchError, UserTurnId,
};
pub use self::report::{FINAL_REPORT_PAGE_BYTES, FinalReport};
pub use self::session::{
    AbortOutcome, END_RECORD_MARGIN, EXIT_GRACE, ExternalAgentSessionSettings,
    FOLLOW_UP_QUEUE_CAPACITY, INTERRUPT_GRACE, PromptAccepted, SKIPPED_LINE_GRACE, SessionPhase,
    SessionRefusal, SessionStep, SessionTotals, SessionView, StreamingBehavior,
    USER_TURNS_PER_TURN_CAPACITY,
};
pub use self::step::ProjectionStep;
pub use self::telemetry::{SessionRecord, TOOL_NAME_RECORD_BYTES};
pub use self::turn::{TurnOutcome, TurnWarning};
