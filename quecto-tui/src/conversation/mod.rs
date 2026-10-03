//! Conversation ownership for `quecto-tui`.
//!
//! Owns master-history pagination, transcript recovery, rewind state, the
//! images attached to the message being composed, and the conversation
//! controllers. `shell::app` composes those controllers as App extensions
//! without taking ownership of conversation policy.

pub(crate) mod history_paging;
pub(crate) mod image_attachments;
pub(crate) mod turn_recovery;
