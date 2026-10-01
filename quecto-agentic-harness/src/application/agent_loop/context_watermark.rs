//! The watermark context's pass (SPIKE, spike/watermark-context): in
//! watermark mode no earlier message is ever edited in place. Before a
//! request the context either stays as it is, or, at the high mark, is
//! cut once down to the low mark (`domain::conversation::watermark`), the
//! cut messages archived to session memory under one index entry.

use super::{ContextManager, ContextPlan};
use crate::application::context_pruning;
use crate::domain::conversation::watermark::{
    ContextWatermark, apply_cut, archive_index, archive_stub, plan_cut,
};
use crate::domain::message::Message;
use crate::domain::session::SpillEntry;

/// The archive index's base id; later cuts are `archive:2`, `archive:3`...
const ARCHIVE_ID: &str = "archive";

impl ContextManager {
    /// Switch the watermark context on (`Some`) or off (`None`).
    pub fn set_context_watermark(&mut self, watermark: Option<ContextWatermark>) {
        self.watermark = watermark;
    }

    pub fn context_watermark(&self) -> Option<ContextWatermark> {
        self.watermark
    }

    /// The watermark pass, when the mode is on: `None` means the default
    /// pruning applies. `fixed_tokens` are the tool definitions' estimate.
    pub async fn prepare_watermark_context(
        &self,
        messages: &mut Vec<Message>,
        fixed_tokens: usize,
        spills_dirty: bool,
    ) -> Option<ContextPlan> {
        let watermark = self.watermark?;
        let tokens_before = context_pruning::estimate_total_tokens(messages);
        // Stamps spill ids only: no content changes.
        let message_spilled = self.spill_unspilled_conversation_messages(messages).await;
        let (high, low) = self.watermark_marks_in_estimate_units(watermark, fixed_tokens);
        let mut plan = ContextPlan {
            tokens_before,
            ..ContextPlan::default()
        };
        if let Some(cut) = plan_cut(messages, high, low) {
            let index_id = self.archive(&messages[cut.head_end..cut.tail_start]).await;
            let count = cut.tail_start - cut.head_end;
            let archived = apply_cut(messages, cut, archive_stub(count, index_id.as_deref()));
            debug_assert_eq!(archived.len(), count);
            plan.messages_dropped = archived.len();
            plan.dropped_calls = archived.into_iter().flat_map(|m| m.tool_calls).collect();
            plan.over_budget = cut.over_low;
            tracing::info!(
                target: "context_prune",
                archived = count,
                high,
                low,
                tokens_before,
                tokens_after = context_pruning::estimate_total_tokens(messages),
                "watermark cut"
            );
        }
        let manifest_shifted = match self.retention.as_ref() {
            Some(retention) if spills_dirty || message_spilled => {
                context_pruning::update_spill_manifest(messages, &retention.list, &self.session_key)
                    .await
            }
            Some(_) | None => false,
        };
        plan.durable_prefix_dirty = manifest_shifted || plan.messages_dropped > 0;
        plan.total_tokens = context_pruning::estimate_total_tokens(messages);
        Some(plan)
    }

    /// The marks for the messages alone, in estimate units: each mark at
    /// the provider-observed scale, less the tool definitions, and the high
    /// mark never above the model's window.
    fn watermark_marks_in_estimate_units(
        &self,
        watermark: ContextWatermark,
        fixed_tokens: usize,
    ) -> (usize, usize) {
        let scale = self.estimate_scale();
        let mut high = scale.in_estimate_units(watermark.high_tokens());
        if let Some(window) = self.window_in_estimate_units() {
            high = high.min(window);
        }
        let low = scale.in_estimate_units(watermark.low_tokens()).min(high);
        (
            high.saturating_sub(fixed_tokens),
            low.saturating_sub(fixed_tokens),
        )
    }

    /// Write the index of `archived` to session memory; its id, or `None`
    /// when nothing retains context or the write failed.
    async fn archive(&self, archived: &[Message]) -> Option<String> {
        let retention = self.retention.as_ref()?;
        let content = archive_index(archived);
        let mut entry = SpillEntry {
            id: ARCHIVE_ID.to_string(),
            tool: ARCHIVE_ID.to_string(),
            input_preview: format!("{} archived messages", archived.len()),
            tokens: context_pruning::estimate_tokens(&content),
            content,
        };
        match retention
            .retain
            .retain_deduplicated(&self.session_key, &mut entry)
            .await
        {
            Ok(retained) => Some(retained.id),
            Err(e) => {
                tracing::warn!(target: "context_prune", error = %e, "failed to archive a watermark cut");
                None
            }
        }
    }
}
