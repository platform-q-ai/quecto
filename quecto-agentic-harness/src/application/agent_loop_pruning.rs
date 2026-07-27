use super::*;

impl AgentLoopImpl {
    pub(super) async fn apply_context_pruning(
        &self,
        messages: &mut Vec<Message>,
        current_turn: u32,
        spills_dirty: bool,
    ) -> usize {
        let plan = self
            .context_manager
            .prepare_provider_context(messages, current_turn, spills_dirty)
            .await;
        let budget = self.effective_max_context_tokens();
        if plan.over_budget {
            // The pinned/exempt set alone exceeds the budget (#1044 AC1).
            tracing::warn!(
                target: "context_prune",
                budget,
                total_tokens = context_pruning::estimate_total_tokens(messages),
                turn = current_turn,
                "context ceiling unmet: the pinned set alone exceeds the budget"
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
                turn = current_turn,
                total_tokens = plan.total_tokens,
                "context pruned"
            );
            self.audit(
                current_turn,
                AuditEvent::ContextPruned {
                    messages_dropped: plan.messages_dropped,
                    tool_results_collapsed: plan.tool_results_collapsed,
                    tokens_before: plan.tokens_before,
                    tokens_after: plan.total_tokens,
                    budget_unmet: plan.over_budget,
                },
            )
            .await;
        }
        plan.total_tokens
    }

    pub async fn prune_resumed_context(&self, messages: &mut Vec<Message>) -> usize {
        self.apply_context_pruning(messages, 0, true).await
    }
}

// #1044/#1045/#1046: context-management tests (750-line cap: separate file).
#[cfg(test)]
#[path = "agent_loop_ctx_mgmt_tests.rs"]
mod ctx_mgmt_tests;
