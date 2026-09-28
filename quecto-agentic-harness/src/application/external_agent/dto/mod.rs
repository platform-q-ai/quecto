//! The external-agent capability's own DTOs (#2285): what the projection
//! answers. The interface layer renders them on the wire.

pub mod audit;
pub mod background;
pub mod message;
pub mod report;
pub mod session;
pub mod turn;

pub use audit::GuardrailDenial;
pub use background::BackgroundJob;
pub use message::{ExecutionState, MessageRole, ProjectedMessage, ProjectedToolCall};
pub use report::{FINAL_REPORT_PAGE_BYTES, FinalReport};
pub use session::SessionTotals;
pub use turn::TurnOutcome;
