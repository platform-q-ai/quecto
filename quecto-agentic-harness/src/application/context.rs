//! Context-management application boundary.
//!
//! Phase 1 hardening groups prompt-context decisions behind this facade so the
//! agent turn loop coordinates the boundary instead of assembling pruning,
//! spill, durable-prefix, and user-facing gauge concerns inline.
//!
//! Invariants owned by this boundary:
//!
//! - the context only ever grows at its end: no earlier message is edited
//!   in place, except by one watermark cut (#2403), or by the emergency
//!   ladder when no cut can bring a request under the ceiling (#2414);
//! - tool-call/tool-result coherence is preserved when messages are cut,
//!   stubbed or dropped, so provider payloads never orphan tool results;
//! - spill/recall promises are maintained by spilling conversation/tool output
//!   before recall stubs are minted, and by avoiding stubs for unspilled
//!   content;
//! - provider-truth context gauges supersede local estimates, while subsequent
//!   estimate-only deltas carry that truth forward until the next provider
//!   observation;
//! - the ceiling decides on calibrated occupancy: the marks and the ceiling
//!   are converted to estimate units at the last provider-observed ratio
//!   (#2212), so the per-message sums stay consistent with them; and
//! - durable prefix dirty semantics are latched for every persisted-layout or
//!   in-place history mutation, including manifest insert/remove, a cut,
//!   stub demotion, and physical drops.

use crate::application::context_pruning;
use crate::application::sessions::use_cases::{ListRetainedContext, RetainContext};
use crate::domain::conversation::watermark::Watermark;
use crate::domain::inference::services::context_calibration::EstimateScale;
use crate::domain::sessions::entities::session_identity::SessionIdentity;
use crate::domain::{catalogue::ModelWindow, message::Message};
use std::sync::{Arc, Mutex};

// #2212: the provider-calibrated gauge and the estimate scale it observes.
#[path = "context_gauge.rs"]
mod gauge;
use gauge::ContextGaugeCalibration;
// #2342: a swarm member's lower ceiling, imposed after construction.
#[path = "context_ceiling_cap.rs"]
mod ceiling_cap;
pub use ceiling_cap::ContextCeilingCap;
// The spill writers (split out for the decrease-only line ceiling, #2342).
#[path = "context_spill_writers.rs"]
mod spill;
// The plan and the tool-message input (split out likewise, #2348).
#[path = "context_plan.rs"]
mod plan;
pub(crate) use plan::{ContextPlan, ToolMessageBuild};
// #2403: the watermark context's pass, beside the agent loop: the only
// context mode (#2414).
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
    pub max_context_tokens: usize,
    /// The emergency ladder's pinned recent turns (#1045).
    pub pin_recent_turns: u32,
    /// The watermark marks (#2403), before the ceiling scales them.
    pub context_marks: Watermark,
    pub model_context_window: Option<usize>,
}

pub(crate) struct ContextManager {
    retention: Option<ContextRetention>,
    session_key: SessionIdentity,
    max_context_tokens: usize,
    pin_recent_turns: u32,
    model_window: ModelWindow,
    ceiling_cap: ContextCeilingCap,
    gauge: Mutex<ContextGaugeCalibration>,
    watermark: watermark::WatermarkState,
}

impl ContextManager {
    pub fn new(config: ContextManagerConfig) -> Self {
        Self {
            retention: config.retention,
            session_key: config.session_key,
            max_context_tokens: config.max_context_tokens,
            pin_recent_turns: config.pin_recent_turns,
            model_window: ModelWindow {
                window: config.model_context_window,
                ..ModelWindow::default()
            },
            ceiling_cap: ContextCeilingCap::default(),
            gauge: Mutex::new(ContextGaugeCalibration::default()),
            watermark: watermark::WatermarkState::new(config.context_marks),
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

    /// The model's window and what its reply needs beside the prompt
    /// (#2405), set together so they never diverge.
    pub fn set_model_window(&mut self, model_window: ModelWindow) {
        self.model_window = model_window;
    }

    #[cfg(test)]
    pub fn set_pin_recent_turns(&mut self, pin_recent_turns: u32) {
        self.pin_recent_turns = pin_recent_turns;
    }

    /// The config-threaded knobs `(pin_recent_turns, marks)`, for wiring
    /// checks (#1045, #2414).
    #[cfg(test)]
    pub fn context_knob_snapshot(&self) -> (u32, Watermark) {
        (self.pin_recent_turns, self.watermark.marks())
    }

    /// The budget clamped to the model's whole window, without a swarm cap or
    /// the reply's reserve: the model's room, not what the member keeps.
    pub fn window_budget_tokens(&self) -> usize {
        self.model_window.budget(self.max_context_tokens)
    }

    /// The lowest of the configured budget, the window less the reply (#2405)
    /// and the composition's cap (a swarm member's, #2342), in provider tokens.
    pub fn effective_max_context_tokens(&self) -> usize {
        let ceiling = self.model_window.ceiling(self.max_context_tokens);
        ceiling.min(self.ceiling_cap.tokens())
    }

    /// The effective budget in estimate units at the provider-observed
    /// scale (#2212), so the cut and the ladder decide on calibrated occupancy.
    pub fn pruning_ceiling_in_estimate_units(&self) -> usize {
        let effective = self.effective_max_context_tokens();
        let ceiling = self.estimate_scale().in_estimate_units(effective);
        debug_assert!(ceiling <= effective, "calibration only tightens");
        ceiling
    }

    /// The prompt's hard limit (the window less the reply, #2405) in estimate units.
    pub fn window_in_estimate_units(&self) -> Option<usize> {
        let room = self.model_window.prompt_room();
        room.map(|room| self.estimate_scale().in_estimate_units(room))
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
}

#[cfg(test)]
#[path = "context_manager_tests.rs"]
mod tests;
