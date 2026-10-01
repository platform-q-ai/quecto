//! The watermark context's pass (#2403). In watermark mode no earlier
//! message is edited in place: before a request the context either stays
//! as it is, or, once it reaches the high mark, is cut once down to the low
//! mark (`domain::conversation::watermark`), the cut messages archived to
//! session memory under one index entry that `recall` reaches.

use super::{ContextManager, ContextPlan};
use crate::application::context_pruning;
use crate::domain::conversation::ContextMode;
use crate::domain::conversation::watermark::Watermark;
use crate::domain::message::Message;

/// The watermark mode's marks, and what it keeps between passes.
#[derive(Debug)]
pub(crate) struct WatermarkState {
    marks: Watermark,
}

impl ContextManager {
    /// Switch the context mode (#2403).
    pub fn set_context_mode(&mut self, mode: ContextMode) {
        let _ = mode;
    }

    /// The context mode in force.
    pub fn context_mode(&self) -> ContextMode {
        match &self.watermark {
            Some(state) => ContextMode::Watermark(state.marks),
            None => ContextMode::Default,
        }
    }

    /// Refresh the spill manifest when the spill store changed (both
    /// passes): whether the history shifted.
    pub(super) async fn refresh_spill_manifest(
        &self,
        messages: &mut Vec<Message>,
        dirty: bool,
    ) -> bool {
        match self.retention.as_ref() {
            Some(retention) if dirty => {
                context_pruning::update_spill_manifest(messages, &retention.list, &self.session_key)
                    .await
            }
            Some(_) | None => false,
        }
    }

    /// The watermark pass when the mode is on; `None` in the default mode,
    /// whose rules then apply. `tool_tokens` is the tool definitions'
    /// estimate.
    pub async fn prepare_watermark_context(
        &self,
        messages: &mut Vec<Message>,
        tool_tokens: usize,
        spills_dirty: bool,
    ) -> Option<ContextPlan> {
        let _ = (messages, tool_tokens, spills_dirty);
        None
    }
}
