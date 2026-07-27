//! Context-management application boundary.
//!
//! Phase 1 hardening groups prompt-context decisions behind this facade so the
//! agent turn loop coordinates the boundary instead of assembling pruning,
//! spill, durable-prefix, and user-facing gauge concerns inline.
//!
//! Invariants owned by this boundary:
//!
//! - pinned recent turns, system prompts, manifests, and the in-flight user
//!   prompt are protected from count-collapse and ceiling demotion;
//! - tool-call/tool-result coherence is preserved when messages are collapsed
//!   or dropped, so provider payloads never orphan tool results;
//! - spill/recall promises are maintained by spilling conversation/tool output
//!   before recall stubs are minted, and by avoiding stubs for unspilled
//!   content;
//! - provider-truth context gauges supersede local estimates, while subsequent
//!   estimate-only pruning deltas carry that truth forward until the next
//!   provider observation; and
//! - durable prefix dirty semantics are latched for every persisted-layout or
//!   in-place history mutation, including manifest insert/remove, stub demotion,
//!   and physical drops.

use crate::application::context_pruning;
use crate::domain::message::{Message, ToolCall};
use crate::domain::session::{ContextSpillStore, SpillEntry};
use crate::domain::tool::ImageBlock;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct ContextGaugeCalibration {
    /// Provider-reported occupancy shown to users after the last exact LLM call,
    /// adjusted by estimated removals/demotions in subsequent pruning passes.
    reported_tokens: usize,
    /// Message-only estimate at the point represented by `reported_tokens`.
    estimated_tokens: usize,
    /// False until a provider supplies usage; without provider truth the gauge
    /// intentionally remains the internal estimate for providers that omit usage.
    has_provider_truth: bool,
}

impl ContextGaugeCalibration {
    pub(super) fn reconcile_before_call(&mut self, current_estimate: usize) -> usize {
        if self.has_provider_truth {
            if current_estimate < self.estimated_tokens {
                self.reported_tokens = self
                    .reported_tokens
                    .saturating_sub(self.estimated_tokens - current_estimate);
            } else if current_estimate > self.estimated_tokens {
                self.reported_tokens = self
                    .reported_tokens
                    .saturating_add(current_estimate - self.estimated_tokens);
            }
            self.estimated_tokens = current_estimate;
            self.reported_tokens
        } else {
            self.estimated_tokens = current_estimate;
            self.reported_tokens = current_estimate;
            current_estimate
        }
    }

    pub(super) fn observe_provider_truth(
        &mut self,
        reported_tokens: usize,
        estimate_at_call: usize,
    ) {
        self.reported_tokens = reported_tokens;
        self.estimated_tokens = estimate_at_call;
        self.has_provider_truth = true;
    }

    pub(super) fn observe_estimate_only(&mut self, estimate: usize) {
        if !self.has_provider_truth {
            self.reported_tokens = estimate;
            self.estimated_tokens = estimate;
        }
    }
}

pub struct ContextManagerConfig {
    pub spill_store: Option<Arc<dyn ContextSpillStore>>,
    pub session_key: String,
    pub context_collapse_after_tool_calls: u32,
    pub max_context_tokens: usize,
    pub pin_recent_turns: u32,
    pub context_collapse_after_messages: u32,
    pub model_context_window: Option<usize>,
}

pub(crate) struct ContextManager {
    spill_store: Option<Arc<dyn ContextSpillStore>>,
    session_key: String,
    context_collapse_after_tool_calls: u32,
    max_context_tokens: usize,
    pin_recent_turns: u32,
    context_collapse_after_messages: u32,
    model_context_window: Option<usize>,
    gauge: Mutex<ContextGaugeCalibration>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ContextPlan {
    pub tokens_before: usize,
    pub total_tokens: usize,
    pub tool_results_collapsed: usize,
    pub messages_stubbed: usize,
    pub messages_dropped: usize,
    pub over_budget: bool,
    pub durable_prefix_dirty: bool,
}

pub(crate) struct ToolMessageBuild<'a> {
    pub tc: &'a ToolCall,
    pub content: String,
    pub image_blocks: Vec<ImageBlock>,
    pub is_error: bool,
}

impl ContextManager {
    pub fn new(config: ContextManagerConfig) -> Self {
        Self {
            spill_store: config.spill_store,
            session_key: config.session_key,
            context_collapse_after_tool_calls: config.context_collapse_after_tool_calls,
            max_context_tokens: config.max_context_tokens,
            pin_recent_turns: config.pin_recent_turns,
            context_collapse_after_messages: config.context_collapse_after_messages,
            model_context_window: config.model_context_window,
            gauge: Mutex::new(ContextGaugeCalibration::default()),
        }
    }

    pub fn set_session_key(&mut self, session_key: String) {
        self.session_key = session_key;
    }

    pub fn set_model_context_window(&mut self, model_context_window: Option<usize>) {
        self.model_context_window = model_context_window;
    }

    #[cfg(test)]
    pub fn set_pin_recent_turns(&mut self, pin_recent_turns: u32) {
        self.pin_recent_turns = pin_recent_turns;
    }

    #[cfg(test)]
    pub fn set_context_collapse_after_messages(&mut self, max_messages: u32) {
        self.context_collapse_after_messages = max_messages;
    }

    #[cfg(test)]
    pub fn context_knob_snapshot(&self) -> (u32, u32) {
        (self.pin_recent_turns, self.context_collapse_after_messages)
    }

    pub fn effective_max_context_tokens(&self) -> usize {
        match self.model_context_window {
            Some(window) => self.max_context_tokens.min(window),
            None => self.max_context_tokens,
        }
    }

    pub fn reconcile_context_gauge(&self, estimate: usize) -> usize {
        self.gauge
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .reconcile_before_call(estimate)
    }

    pub fn observe_provider_context_gauge(&self, reported_tokens: usize, estimate_at_call: usize) {
        self.gauge
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .observe_provider_truth(reported_tokens, estimate_at_call);
    }

    pub fn observe_estimated_context_gauge(&self, estimate: usize) {
        self.gauge
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .observe_estimate_only(estimate);
    }

    #[cfg(test)]
    pub fn poison_context_gauge_lock_for_test(&self) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = self.gauge.lock().unwrap();
            panic!("poison context gauge mutex for coverage");
        }));
        assert!(
            self.gauge.is_poisoned(),
            "context gauge mutex must be poisoned after the intentional panic"
        );
    }

    pub fn build_tool_message(&self, args: ToolMessageBuild<'_>) -> Message {
        let mut tool_msg = Message::tool(args.tc.id.clone(), args.content);
        tool_msg.tool_name = Some(args.tc.name.clone());
        tool_msg.input_preview =
            Some(context_pruning::truncate_utf8_safe(&args.tc.arguments, 100).into_owned());
        tool_msg.image_blocks = args.image_blocks;
        tool_msg.invalidate_token_cache();
        tool_msg.is_error = args.is_error;
        tool_msg
    }

    pub async fn spill_tool_message(&self, tool_msg: &mut Message, spill_id: String) {
        let Some(spill_store) = self.spill_store.as_ref() else {
            return;
        };

        let content = std::mem::take(&mut tool_msg.content);
        let entry = SpillEntry {
            id: spill_id,
            tool: tool_msg
                .tool_name
                .clone()
                .unwrap_or_else(|| "tool".to_string()),
            input_preview: tool_msg.input_preview.clone().unwrap_or_default(),
            tokens: context_pruning::estimate_tokens(&content),
            content,
        };
        let result = spill_store.append(&self.session_key, &entry).await;
        tool_msg.content = entry.content;
        tool_msg.invalidate_token_cache();
        match result {
            Ok(()) => tool_msg.spill_id = Some(entry.id),
            Err(e) => {
                tracing::warn!(target: "context_prune", error = %e, "failed to spill tool output");
            }
        }
    }

    pub async fn spill_conversation_message(&self, msg: &mut Message) {
        if let Some(spill_store) = self.spill_store.as_ref() {
            context_pruning::messages::spill_conversation_message(
                msg,
                spill_store.as_ref(),
                &self.session_key,
            )
            .await;
        }
    }

    pub async fn prepare_provider_context(
        &self,
        messages: &mut Vec<Message>,
        _current_turn: u32,
        spills_dirty: bool,
    ) -> ContextPlan {
        let tokens_before = context_pruning::estimate_total_tokens(messages);
        let message_spilled = self.spill_unspilled_conversation_messages(messages).await;
        let collapsed = context_pruning::collapse_tool_results_over_limit(
            messages,
            self.context_collapse_after_tool_calls,
        );
        let msg_collapsed = context_pruning::messages::collapse_conversation_messages_over_limit(
            messages,
            self.context_collapse_after_messages,
            self.pin_recent_turns,
        );
        let outcome = context_pruning::messages::enforce_context_ceiling_ladder(
            messages,
            self.effective_max_context_tokens(),
            self.pin_recent_turns,
        );
        let mut manifest_shifted = false;
        if spills_dirty || message_spilled {
            if let Some(spill_store) = self.spill_store.as_ref() {
                manifest_shifted = context_pruning::update_spill_manifest(
                    messages,
                    spill_store.as_ref(),
                    &self.session_key,
                )
                .await;
            }
        }
        let total_tokens = context_pruning::estimate_total_tokens(messages);
        let messages_stubbed = msg_collapsed + outcome.collapsed_to_stubs;
        let durable_prefix_dirty =
            collapsed > 0 || messages_stubbed > 0 || outcome.dropped > 0 || manifest_shifted;

        ContextPlan {
            tokens_before,
            total_tokens,
            tool_results_collapsed: collapsed,
            messages_stubbed,
            messages_dropped: outcome.dropped,
            over_budget: outcome.over_budget,
            durable_prefix_dirty,
        }
    }

    async fn spill_unspilled_conversation_messages(&self, messages: &mut [Message]) -> bool {
        let Some(spill_store) = self.spill_store.as_ref() else {
            return false;
        };
        let mut spilled = false;
        for msg in messages
            .iter_mut()
            .filter(|m| m.spill_id.is_none() && !m.is_manifest)
        {
            spilled |= context_pruning::messages::spill_conversation_message(
                msg,
                spill_store.as_ref(),
                &self.session_key,
            )
            .await;
        }
        spilled
    }
}

#[cfg(test)]
#[path = "context_manager_tests.rs"]
mod tests;
