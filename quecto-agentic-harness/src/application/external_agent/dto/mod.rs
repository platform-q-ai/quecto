//! The external-agent capability's own DTOs (#2285): what the projection
//! answers. The interface layer renders them on the wire.

pub mod audit;
pub mod background;
pub mod message;
pub mod report;
pub mod session;
pub mod step;
pub mod turn;

pub use self::audit::{
    ADMISSION_WARNING_CAPACITY, AUDIT_INPUT_PREVIEW_BYTES, GUARDRAIL_AUDIT_CAPACITY,
    GuardrailDenial,
};
pub use self::background::{BACKGROUND_JOB_CAPACITY, BackgroundJob, TERMINAL_TASK_STATUSES};
pub use self::message::{
    ExecutionState, MessageRole, ProjectedMessage, ProjectedToolCall, TOOL_RESULT_CONTENT_BYTES,
    TRUNCATION_MARKER,
};
pub use self::report::{FINAL_REPORT_PAGE_BYTES, FinalReport};
pub use self::session::SessionTotals;
pub use self::step::ProjectionStep;
pub use self::turn::{TurnOutcome, TurnWarning};
