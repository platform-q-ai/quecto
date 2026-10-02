use super::*;
use crate::domain::context_calibration::{MessageBudget, message_budget};

impl AgentLoopImpl {
    pub(super) async fn apply_context_pruning(
        &self,
        messages: &mut Vec<Message>,
        current_turn: u32,
        spills_dirty: bool,
    ) -> usize {
        // Every request carries the tool definitions too (#2160): they count
        // against the budget and in the estimate. #2212: all in estimate
        // units at the provider-observed residual scale; the ladder sums
        // per-class estimates against it, the scale applying to all alike.
        self.log_pending_window_note();
        let fixed_tokens = self.tool_definition_tokens();
        let effective = self.context_manager.pruning_ceiling_in_estimate_units();
        let window = self.context_manager.window_in_estimate_units();
        let MessageBudget {
            tokens: budget,
            window_exceeded,
        } = message_budget(effective, window, fixed_tokens);
        // The quarter floor (#2182) keeps a minimum conversation beside a
        // large tool set, so it may pass the configured budget: that budget
        // is soft (the model's window, a hard limit, caps the floor inside
        // `message_budget`). Passing it is reported: the plan is over budget
        // and the ContextPruned audit records `budget_unmet`.
        let floor_overrides = budget.saturating_add(fixed_tokens) > effective;
        if floor_overrides {
            tracing::warn!(
                target: "context_prune",
                fixed_tokens,
                effective,
                budget,
                window_exceeded,
                "the tool definitions take most of the context budget; the messages keep a quarter of it"
            );
        }
        let live_before = live_result_ids(messages);
        // #2403: in watermark mode its pass replaces every pruning rule.
        let watermark = self
            .context_manager
            .prepare_watermark_context(messages, (fixed_tokens, budget), spills_dirty)
            .await;
        let (mut plan, cut, watermark_fallback) = match watermark {
            Some(pass) => (pass.plan, pass.cut, pass.fallback),
            None => {
                let plan = self
                    .context_manager
                    .prepare_provider_context(messages, budget, spills_dirty)
                    .await;
                (plan, None, false)
            }
        };
        // #2404: a due watermark cut's record, made or skipped. A cut is
        // never a `context_pruned` record; the emergency ladder's is, marked.
        if let Some(cut) = cut {
            self.audit(current_turn, cut.into_event()).await;
        }
        if plan.durable_prefix_dirty {
            self.report_collapsed_results(messages, &live_before, &plan.dropped_calls);
        }
        // The floor passed the configured budget, or the tools alone fill
        // the model's window (no transcript fits).
        plan.over_budget |= floor_overrides || window_exceeded;
        if plan.over_budget {
            // The pinned/exempt set alone exceeds the budget (#1044 AC1) — the
            // agent's kept report stub among it (#2226) — the floor passed
            // it, or the tool definitions alone fill the window.
            tracing::warn!(
                target: "context_prune",
                budget,
                window_exceeded,
                total_tokens = context_pruning::estimate_total_tokens(messages),
                turn = current_turn,
                "context ceiling unmet: the pinned set (with the kept report) or the tool definitions exceed the budget"
            );
        }
        if plan.durable_prefix_dirty {
            self.latch_durable_prefix_dirty();
        }
        if watermark_fallback
            || plan.tool_results_collapsed > 0
            || plan.messages_collapsed > 0
            || plan.ladder_stubbed > 0
            || plan.messages_dropped > 0
            || plan.snapshots_superseded > 0
            || plan.large_results_collapsed > 0
            || plan.over_budget
        {
            let ceiling_tokens = self.context_manager.effective_max_context_tokens();
            tracing::info!(
                target: "context_prune",
                collapsed = plan.tool_results_collapsed,
                messages_collapsed = plan.messages_collapsed,
                ladder_stubbed = plan.ladder_stubbed,
                dropped = plan.messages_dropped,
                snapshots_superseded = plan.snapshots_superseded,
                large_results_collapsed = plan.large_results_collapsed,
                ceiling_tokens,
                budget_unmet = plan.over_budget,
                watermark_fallback,
                estimate_scale_permille = self.context_manager.estimate_scale().permille(),
                turn = current_turn,
                total_tokens = plan.total_tokens,
                "context pruned"
            );
            self.audit(
                current_turn,
                AuditEvent::ContextPruned {
                    messages_dropped: plan.messages_dropped,
                    tool_results_collapsed: plan.tool_results_collapsed,
                    // Counted as the estimate is: with the tool definitions.
                    tokens_before: plan.tokens_before.saturating_add(fixed_tokens),
                    tokens_after: plan.total_tokens.saturating_add(fixed_tokens),
                    budget_unmet: plan.over_budget,
                    messages_collapsed: plan.messages_collapsed,
                    ladder_stubbed: plan.ladder_stubbed,
                    snapshots_superseded: plan.snapshots_superseded,
                    ceiling_tokens,
                    large_results_collapsed: plan.large_results_collapsed,
                    watermark_fallback,
                },
            )
            .await;
        }
        plan.total_tokens.saturating_add(fixed_tokens)
    }

    /// Tell each tool whose result a rule just collapsed or dropped (#2348
    /// review M1, final review), so a tool answering repeats from what it
    /// delivered (the read cache) forgets that delivery. Only result ids
    /// are recorded before the prune; a dropped result's call comes back
    /// from the ladder, moved out of its message, so no call is cloned (#993).
    fn report_collapsed_results(
        &self,
        messages: &[Message],
        live_before: &std::collections::BTreeSet<String>,
        dropped_calls: &[ToolCall],
    ) {
        let live_after = live_result_ids(messages);
        let remaining = messages.iter().flat_map(|m| &m.tool_calls);
        for call in remaining
            .chain(dropped_calls)
            .filter(|call| live_before.contains(&call.id) && !live_after.contains(&call.id))
        {
            self.tool_executor()
                .result_collapsed(&call.name, &call.arguments);
        }
    }

    /// The estimate of the tool definitions every request carries (#2160).
    pub(super) fn tool_definition_tokens(&self) -> usize {
        self.current_tool_definitions()
            .iter()
            .map(crate::domain::tool::ToolDefinition::estimated_tokens)
            .sum()
    }

    /// Switch the context mode (#2403): the default pruning rules, or the
    /// watermark context.
    pub fn set_context_mode(&mut self, mode: crate::domain::conversation::ContextMode) {
        self.context_manager.set_context_mode(mode);
    }

    /// The context mode in force.
    pub fn context_mode(&self) -> crate::domain::conversation::ContextMode {
        self.context_manager.context_mode()
    }

    pub async fn prune_resumed_context(&self, messages: &mut Vec<Message>) -> usize {
        self.apply_context_pruning(messages, 0, true).await
    }
}

/// The call ids of the tool results still in full.
fn live_result_ids(messages: &[Message]) -> std::collections::BTreeSet<String> {
    messages
        .iter()
        .filter(|m| m.role == crate::domain::message::Role::Tool && !m.is_collapsed)
        .filter_map(|m| m.tool_call_id.clone())
        .collect()
}

// #1044/#1045/#1046: context-management tests (750-line cap: separate file).
#[cfg(test)]
#[path = "agent_loop_ctx_mgmt_tests.rs"]
pub(super) mod ctx_mgmt_tests;

// #2212: provider-calibrated ceiling tests.
#[cfg(test)]
#[path = "agent_loop_calibration_tests.rs"]
mod calibration_tests;

// #2342: a scripted coordinator's request tokens, before and after.
#[cfg(test)]
#[path = "agent_loop_snapshot_tests.rs"]
mod snapshot_tests;

// #2403: the watermark context's pass on the agent loop.
#[cfg(test)]
#[path = "agent_loop/watermark_tests.rs"]
mod watermark_tests;

// #2404: the watermark context's event-log records.
#[cfg(test)]
#[path = "agent_loop/watermark_telemetry_tests.rs"]
mod watermark_telemetry_tests;

// #2403: every request extends the previous one, through the Codex serializer.
#[cfg(test)]
#[path = "agent_loop/watermark_sim_tests.rs"]
mod watermark_sim_tests;

// #2348: the size-aware collapse on the same scripted coordinator.
#[cfg(test)]
#[path = "agent_loop_large_result_tests.rs"]
mod large_result_tests;
