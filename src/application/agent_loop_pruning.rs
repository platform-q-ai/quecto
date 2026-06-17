use super::*;

impl AgentLoopImpl {
    pub(super) async fn apply_context_pruning(
        &self,
        messages: &mut Vec<Message>,
        current_turn: u32,
        spills_dirty: bool,
    ) -> usize {
        // Snapshot token count before pruning for accurate audit logging.
        let tokens_before = context_pruning::estimate_total_tokens(messages);

        // Collapse is disabled by default (COLLAPSE_DISABLED = u32::MAX).
        // Still available for users who explicitly lower the config value.
        let collapsed = if self.context_collapse_after_turns < context_pruning::COLLAPSE_DISABLED {
            context_pruning::collapse_old_tool_results(
                messages,
                current_turn,
                self.context_collapse_after_turns,
            )
        } else {
            0
        };
        let dropped = context_pruning::enforce_context_ceiling(messages, self.max_context_tokens);
        // Only rebuild manifest when spills have changed (new tool results spilled)
        if spills_dirty {
            if let Some(ref spill_store) = self.spill_store {
                context_pruning::update_spill_manifest(
                    messages,
                    spill_store.as_ref(),
                    &self.session_key,
                )
                .await;
            }
        }
        let total_tokens = context_pruning::estimate_total_tokens(messages);
        if collapsed > 0 || dropped > 0 {
            tracing::info!(
                target: "context_prune",
                collapsed,
                dropped,
                turn = current_turn,
                total_tokens,
                "context pruned"
            );
            self.audit(
                current_turn,
                AuditEvent::ContextPruned {
                    messages_dropped: dropped,
                    tool_results_collapsed: collapsed,
                    tokens_before,
                    tokens_after: total_tokens,
                },
            )
            .await;
        }
        total_tokens
    }

    pub async fn prune_resumed_context(&self, messages: &mut Vec<Message>) -> usize {
        self.apply_context_pruning(messages, 0, true).await
    }
}
