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
        let mut plan = self
            .context_manager
            .prepare_provider_context(messages, budget, spills_dirty)
            .await;
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
        if plan.tool_results_collapsed > 0
            || plan.messages_stubbed > 0
            || plan.messages_dropped > 0
            || plan.over_budget
        {
            tracing::info!(
                target: "context_prune",
                collapsed = plan.tool_results_collapsed,
                messages_stubbed = plan.messages_stubbed,
                dropped = plan.messages_dropped,
                budget_unmet = plan.over_budget,
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
                },
            )
            .await;
        }
        plan.total_tokens.saturating_add(fixed_tokens)
    }

    /// The estimate of the tool definitions every request carries (#2160).
    pub(super) fn tool_definition_tokens(&self) -> usize {
        self.current_tool_definitions()
            .iter()
            .map(crate::domain::tool::ToolDefinition::estimated_tokens)
            .sum()
    }

    pub async fn prune_resumed_context(&self, messages: &mut Vec<Message>) -> usize {
        self.apply_context_pruning(messages, 0, true).await
    }
}

// #1044/#1045/#1046: context-management tests (750-line cap: separate file).
#[cfg(test)]
#[path = "agent_loop_ctx_mgmt_tests.rs"]
mod ctx_mgmt_tests;

// #2212: provider-calibrated ceiling tests.
#[cfg(test)]
#[path = "agent_loop_calibration_tests.rs"]
mod calibration_tests;
