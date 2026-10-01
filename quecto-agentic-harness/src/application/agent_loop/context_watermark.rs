//! The watermark context's pass (#2403). In watermark mode no earlier
//! message is edited in place: before a request the context either stays
//! as it is, or, once it reaches the high mark, is cut once down to the low
//! mark (`domain::conversation::watermark`), the cut messages archived to
//! session memory under one index entry that `recall` reaches.
//!
//! The storm guard's baseline (the messages' size right after the last
//! cut) belongs to that cut's stub: once the stub has left the
//! conversation (a clear, a rewind before it, another session) the
//! baseline is reset. It is not saved: a resumed session starts without
//! it, so its first request over H may cut at once, a cut that still has
//! to save a tenth of H.

use super::{ContextManager, ContextPlan};
use crate::application::context_pruning;
use crate::domain::conversation::ContextMode;
use crate::domain::conversation::watermark::{
    CutInput, CutPlan, CutTrigger, RequestSize, Watermark, plan_cut,
};
use crate::domain::conversation::watermark_cut::{
    apply_cut, archive_index, archive_stub, archived, plan_messages, stub_tokens_bound,
};
use crate::domain::message::Message;
use crate::domain::session::SpillEntry;
use std::sync::Mutex;

/// The archive index's base id; later cuts are `archive:2`, `archive:3`...
const ARCHIVE_ID: &str = "archive";

/// The watermark mode's marks, and the last cut it made.
#[derive(Debug)]
pub(crate) struct WatermarkState {
    marks: Watermark,
    last_cut: Mutex<Option<LastCut>>,
}

/// The baseline a cut leaves the storm guard, and the stub that marks it.
#[derive(Debug, Clone, Copy)]
struct LastCut {
    stub: uuid::Uuid,
    message_tokens: usize,
}

impl ContextManager {
    /// Switch the context mode (#2403); a switch forgets any previous cut.
    pub fn set_context_mode(&mut self, mode: ContextMode) {
        self.watermark = match mode {
            ContextMode::Watermark(marks) => Some(WatermarkState {
                marks,
                last_cut: Mutex::new(None),
            }),
            ContextMode::Default => None,
        };
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
    /// estimate. Every await comes before the cut, which is made in one
    /// step: a pass dropped while it archives leaves the conversation as
    /// it was.
    pub async fn prepare_watermark_context(
        &self,
        messages: &mut Vec<Message>,
        tool_tokens: usize,
        spills_dirty: bool,
    ) -> Option<ContextPlan> {
        let state = self.watermark.as_ref()?;
        let tokens_before = context_pruning::estimate_total_tokens(messages);
        // Stamps spill ids only, so every archived message is recallable.
        let message_spilled = self.spill_unspilled_conversation_messages(messages).await;
        let manifest_shifted = self
            .refresh_spill_manifest(messages, spills_dirty || message_spilled)
            .await;
        let trigger = self.cut_trigger(state, messages);
        let size = RequestSize {
            tool_tokens,
            message_tokens: context_pruning::estimate_total_tokens(messages),
        };
        let mut plan = ContextPlan {
            tokens_before,
            ..ContextPlan::default()
        };
        if trigger.is_due(size) {
            self.cut(state, trigger, messages, tool_tokens, &mut plan)
                .await;
        }
        plan.total_tokens = context_pruning::estimate_total_tokens(messages);
        plan.over_budget = plan.total_tokens.saturating_add(tool_tokens) > trigger.ceiling;
        plan.durable_prefix_dirty = manifest_shifted || plan.messages_dropped > 0;
        Some(plan)
    }

    /// The trigger in estimate units: the marks at the provider-observed
    /// scale under the ceiling, and the last cut's baseline while its stub
    /// is still in the conversation (reset, and forgotten, once it is not).
    fn cut_trigger(&self, state: &WatermarkState, messages: &[Message]) -> CutTrigger {
        let scale = self.estimate_scale();
        let marks = state
            .marks
            .under_ceiling(scale.in_estimate_units(state.marks.high()));
        let trigger = CutTrigger {
            marks,
            ceiling: self.pruning_ceiling_in_estimate_units(),
            after_last_cut: None,
        };
        let mut last_cut = state.last_cut.lock().unwrap_or_else(|e| e.into_inner());
        let in_force = last_cut.filter(|cut| messages.iter().any(|m| m.id() == cut.stub));
        match in_force {
            Some(cut) => CutTrigger {
                after_last_cut: Some(cut.message_tokens),
                ..trigger
            },
            None => {
                *last_cut = None;
                trigger.reset()
            }
        }
    }

    /// Plan the cut and, when there is one, archive what it drops and make
    /// it; the outcome goes into `plan`.
    async fn cut(
        &self,
        state: &WatermarkState,
        trigger: CutTrigger,
        messages: &mut Vec<Message>,
        tool_tokens: usize,
        plan: &mut ContextPlan,
    ) {
        let view = plan_messages(messages);
        let input = CutInput {
            messages: &view,
            tool_tokens,
            stub_tokens: stub_tokens_bound(),
            trigger,
        };
        let cut = match plan_cut(&input) {
            Ok(cut) => cut,
            Err(reason) => {
                tracing::debug!(target: "context_prune", %reason, "watermark cut not made");
                return;
            }
        };
        let index = archive_index(&archived(messages, &cut));
        let count = cut.archived().iter().map(|range| range.len()).sum();
        let index_id = self.archive(index, count).await;
        // From here on nothing awaits: the cut is made whole or not at all.
        let stub = archive_stub(count, index_id.as_deref());
        assert!(
            stub.estimated_tokens() <= stub_tokens_bound(),
            "the stub's bound holds"
        );
        let stub_id = stub.id();
        let dropped = apply_cut(messages, &cut, stub);
        *state.last_cut.lock().unwrap_or_else(|e| e.into_inner()) = Some(LastCut {
            stub: stub_id,
            message_tokens: baseline(trigger, &cut),
        });
        plan.messages_dropped = dropped.len();
        plan.dropped_calls = dropped.into_iter().flat_map(|m| m.tool_calls).collect();
        tracing::info!(
            target: "context_prune",
            archived = count,
            index = index_id.as_deref().unwrap_or("none"),
            high = trigger.effective_marks().high(),
            low = trigger.effective_marks().low(),
            projected_tokens = cut.projected_tokens(),
            fill = ?cut.fill(),
            "watermark cut"
        );
    }

    /// Write `index` to session memory: its id, or `None` when nothing
    /// retains context or the write failed (the stub then says so).
    async fn archive(&self, index: String, count: usize) -> Option<String> {
        let retention = self.retention.as_ref()?;
        let mut entry = SpillEntry {
            id: ARCHIVE_ID.to_string(),
            tool: ARCHIVE_ID.to_string(),
            input_preview: format!("{count} archived messages"),
            tokens: context_pruning::estimate_tokens(&index),
            content: index,
        };
        match retention
            .retain
            .retain_deduplicated(&self.session_key, &mut entry)
            .await
        {
            Ok(retained) => Some(retained.id),
            Err(error) => {
                tracing::warn!(target: "context_prune", %error, "failed to archive a watermark cut");
                None
            }
        }
    }
}

/// The storm guard's baseline after `cut`.
fn baseline(trigger: CutTrigger, cut: &CutPlan) -> usize {
    let after = trigger.after_cut(cut).after_last_cut;
    after.expect("a cut sets the baseline")
}
