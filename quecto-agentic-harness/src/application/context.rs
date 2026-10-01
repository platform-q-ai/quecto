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
//!   provider observation;
//! - the ceiling decides on calibrated occupancy: the effective budget is
//!   converted to estimate units at the last provider-observed ratio
//!   (#2212), so the ladder's per-message sums stay consistent with it;
//! - the newest whole snapshot of a state (a full swarm summary) stays in
//!   full at every dial, and every older one it supersedes is collapsed to
//!   its recall stub (#2342);
//! - a large tool result the model has seen for a few turns collapses to
//!   its recall stub whatever the count dials say (#2348); and
//! - durable prefix dirty semantics are latched for every persisted-layout or
//!   in-place history mutation, including manifest insert/remove, stub demotion,
//!   and physical drops.

use crate::application::context_pruning;
use crate::application::sessions::use_cases::{ListRetainedContext, RetainContext};
use crate::domain::context_calibration::EstimateScale;
use crate::domain::large_result_collapse::LargeResultCollapse;
use crate::domain::message::Message;
use crate::domain::session_identity::SessionIdentity;
use std::sync::{Arc, Mutex};

// #2212: the provider-calibrated gauge and the estimate scale it observes.
#[path = "context_gauge.rs"]
mod gauge;
use gauge::ContextGaugeCalibration;
// #2342: a swarm member's lower ceiling, imposed after construction.
#[path = "context_ceiling_cap.rs"]
mod ceiling_cap;
pub use ceiling_cap::ContextCeilingCap;
// #2348 review M1: a swarm member's size-aware collapse, engaged likewise.
#[path = "context_large_result_switch.rs"]
mod large_result_switch;
pub use large_result_switch::LargeResultSwitch;
// The spill writers (split out for the decrease-only line ceiling, #2342).
#[path = "context_spill_writers.rs"]
mod spill;
// The plan and the tool-message input (split out likewise, #2348).
#[path = "context_plan.rs"]
mod plan;
pub(crate) use plan::{ContextPlan, ToolMessageBuild};
#[path = "agent_loop/context_watermark.rs"]
mod watermark;

/// The narrow handles the pruning policy holds on the sessions
/// capability's retention namespace (D9 #1978): the writer that appends
/// an entry under the caller's id and the reader that says whether the
/// namespace holds anything. Composition builds both over the one
/// retention store; the policy never reaches the store, its file or its
/// cache. `None` on the loop config means the loop retains nothing.
#[derive(Clone)]
pub struct ContextRetention {
    pub retain: Arc<RetainContext>,
    pub list: Arc<ListRetainedContext>,
}

impl std::fmt::Debug for ContextRetention {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextRetention").finish_non_exhaustive()
    }
}

pub struct ContextManagerConfig {
    pub retention: Option<ContextRetention>,
    pub session_key: SessionIdentity,
    pub context_collapse_after_tool_calls: u32,
    pub max_context_tokens: usize,
    pub pin_recent_turns: u32,
    pub context_collapse_after_messages: u32,
    pub large_result_collapse: LargeResultCollapse,
    pub model_context_window: Option<usize>,
}

pub(crate) struct ContextManager {
    retention: Option<ContextRetention>,
    session_key: SessionIdentity,
    context_collapse_after_tool_calls: u32,
    max_context_tokens: usize,
    pin_recent_turns: u32,
    context_collapse_after_messages: u32,
    large_result_collapse: LargeResultSwitch,
    model_context_window: Option<usize>,
    ceiling_cap: ContextCeilingCap,
    gauge: Mutex<ContextGaugeCalibration>,
    watermark: Option<crate::domain::conversation::watermark::ContextWatermark>,
}

impl ContextManager {
    pub fn new(config: ContextManagerConfig) -> Self {
        Self {
            retention: config.retention,
            session_key: config.session_key,
            context_collapse_after_tool_calls: config.context_collapse_after_tool_calls,
            max_context_tokens: config.max_context_tokens,
            pin_recent_turns: config.pin_recent_turns,
            context_collapse_after_messages: config.context_collapse_after_messages,
            large_result_collapse: LargeResultSwitch::new(config.large_result_collapse),
            model_context_window: config.model_context_window,
            ceiling_cap: ContextCeilingCap::default(),
            gauge: Mutex::new(ContextGaugeCalibration::default()),
            watermark: None,
        }
    }

    /// Another session's transcript: the last observation does not
    /// describe it, so the gauge and its scale start over (#2212).
    pub fn set_session_key(&mut self, session_key: SessionIdentity) {
        self.session_key = session_key;
        self.gauge
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .forget_calibration();
    }

    /// The handle the composition lowers the ceiling through (#2342).
    pub fn ceiling_cap(&self) -> ContextCeilingCap {
        self.ceiling_cap.clone()
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

    /// The configured budget clamped to the model's window, without a swarm
    /// member's cap: the room the model has, not what the member keeps.
    pub fn window_budget_tokens(&self) -> usize {
        match self.model_context_window {
            Some(window) => self.max_context_tokens.min(window),
            None => self.max_context_tokens,
        }
    }

    /// The lowest of the configured budget, the model's window and the
    /// composition's cap (a swarm member's, #2342), in provider tokens.
    pub fn effective_max_context_tokens(&self) -> usize {
        self.window_budget_tokens().min(self.ceiling_cap.tokens())
    }

    /// The effective budget in estimate units at the provider-observed
    /// scale (#2212), so the ladder prunes on calibrated occupancy.
    pub fn pruning_ceiling_in_estimate_units(&self) -> usize {
        let effective = self.effective_max_context_tokens();
        let ceiling = self.estimate_scale().in_estimate_units(effective);
        debug_assert!(ceiling <= effective, "calibration only tightens");
        ceiling
    }

    /// The model's window (a hard provider limit) in estimate units.
    pub fn window_in_estimate_units(&self) -> Option<usize> {
        let scale = self.estimate_scale();
        self.model_context_window
            .map(|window| scale.in_estimate_units(window))
    }

    pub fn estimate_scale(&self) -> EstimateScale {
        self.gauge
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .estimate_scale()
    }

    /// Forget the provider figure and the scale (a model or provider
    /// change: another tokeniser; see `ContextGaugeCalibration`).
    pub fn forget_calibration(&self) {
        self.gauge
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .forget_calibration();
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

    pub async fn prepare_provider_context(
        &self,
        messages: &mut Vec<Message>,
        message_budget: usize,
        spills_dirty: bool,
    ) -> ContextPlan {
        let tokens_before = context_pruning::estimate_total_tokens(messages);
        let message_spilled = self.spill_unspilled_conversation_messages(messages).await;
        // Superseded snapshots go first (#2342): the dials then count and
        // weigh only what is still current.
        let superseded = context_pruning::snapshots::collapse_superseded_snapshots(messages);
        // A large result the model has seen for a few turns goes to its
        // stub whatever the count dials say (#2348).
        let large = context_pruning::large_results::collapse_large_results(
            messages,
            self.large_result_collapse.dial(),
        );
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
            message_budget,
            self.pin_recent_turns,
        );
        let mut manifest_shifted = false;
        if spills_dirty || message_spilled {
            if let Some(retention) = self.retention.as_ref() {
                manifest_shifted = context_pruning::update_spill_manifest(
                    messages,
                    &retention.list,
                    &self.session_key,
                )
                .await;
            }
        }
        let durable_prefix_dirty = manifest_shifted
            || superseded
                + large
                + collapsed
                + msg_collapsed
                + outcome.collapsed_to_stubs
                + outcome.dropped
                > 0;
        ContextPlan {
            tokens_before,
            total_tokens: context_pruning::estimate_total_tokens(messages),
            tool_results_collapsed: collapsed,
            messages_collapsed: msg_collapsed,
            ladder_stubbed: outcome.collapsed_to_stubs,
            messages_dropped: outcome.dropped,
            dropped_calls: outcome.dropped_calls,
            snapshots_superseded: superseded,
            large_results_collapsed: large,
            over_budget: outcome.over_budget,
            durable_prefix_dirty,
        }
    }
}

#[cfg(test)]
#[path = "context_manager_tests.rs"]
mod tests;
