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

        // Creation-time spill (#1046 AC1): every conversation message not yet
        // on disk (in-flight user prompts, resumed pre-#1046 histories) is
        // filed through the single spill writer, so later demotion never has
        // to write content itself.
        let message_spilled = self.spill_unspilled_conversation_messages(messages).await;

        // Collapse the oldest tool results once the session exceeds
        // `context_collapse_after_tool_calls` tool calls (default 50). The
        // sentinel COLLAPSE_DISABLED (u32::MAX) disables collapse; the callee
        // short-circuits on it, so no guard is needed here.
        let collapsed = context_pruning::collapse_tool_results_over_limit(
            messages,
            self.context_collapse_after_tool_calls,
        );
        // Count-based conversation-message collapse (#1046 AC2) — the
        // independent dial symmetric with the tool-call trigger above.
        let msg_collapsed = context_pruning::messages::collapse_conversation_messages_over_limit(
            messages,
            self.context_collapse_after_messages,
            self.pin_recent_turns,
        );
        // Enforce the token ceiling down the demotion ladder (#1046 AC6):
        // stub first, drop stubs only if still over budget. The budget is the
        // window-aware effective value (#1044 AC2).
        let budget = self.effective_max_context_tokens();
        let outcome = context_pruning::messages::enforce_context_ceiling_ladder(
            messages,
            budget,
            self.pin_recent_turns,
        );
        if outcome.over_budget {
            // The pinned/exempt set alone exceeds the budget (#1044 AC1).
            tracing::warn!(
                target: "context_prune",
                budget,
                total_tokens = context_pruning::estimate_total_tokens(messages),
                turn = current_turn,
                "context ceiling unmet: the pinned set alone exceeds the budget"
            );
        }
        // Rebuild the manifest when spills have changed: new tool results
        // spilled last turn, or conversation messages just spilled (which can
        // happen on a turn with no tool calls at all).
        if spills_dirty || message_spilled {
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
        let stubbed = msg_collapsed + outcome.collapsed_to_stubs;
        // #1072: any mutating outcome — an in-place collapse/stub demotion
        // (content changes, message ids do NOT) or a physical drop — means the
        // live history is no longer an append-only extension of what was
        // already persisted, so latch the durable-prefix dirty flag. The latch
        // is outcome-independent: it survives an Error or Cancelled turn.
        if collapsed > 0 || stubbed > 0 || outcome.dropped > 0 {
            self.latch_durable_prefix_dirty();
        }
        if collapsed > 0 || stubbed > 0 || outcome.dropped > 0 || outcome.over_budget {
            tracing::info!(
                target: "context_prune",
                collapsed,
                messages_stubbed = stubbed,
                dropped = outcome.dropped,
                budget_unmet = outcome.over_budget,
                turn = current_turn,
                total_tokens,
                "context pruned"
            );
            self.audit(
                current_turn,
                AuditEvent::ContextPruned {
                    messages_dropped: outcome.dropped,
                    tool_results_collapsed: collapsed,
                    tokens_before,
                    tokens_after: total_tokens,
                    budget_unmet: outcome.over_budget,
                },
            )
            .await;
        }
        total_tokens
    }

    /// File every not-yet-spilled conversation message through the single
    /// spill writer (#1046 AC1). Returns true when anything was written.
    async fn spill_unspilled_conversation_messages(&self, messages: &mut [Message]) -> bool {
        // Ephemeral sessions (empty key) spill to disk too — deliberately, so
        // collapse/ladder recall() stubs stay resolvable within the run (see
        // the NOTE in agent_loop_spill.rs). The ephemeral interface paths
        // scrub the empty-key spill file at run end, so nothing persists
        // beyond the run.
        let Some(ref spill_store) = self.spill_store else {
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

    pub async fn prune_resumed_context(&self, messages: &mut Vec<Message>) -> usize {
        self.apply_context_pruning(messages, 0, true).await
    }
}

// #1044/#1045/#1046: context-management tests (750-line cap: separate file).
#[cfg(test)]
#[path = "agent_loop_ctx_mgmt_tests.rs"]
mod ctx_mgmt_tests;
