//! Spill-writer plumbing for the agent loop: tool-result message assembly +
//! creation-time spilling of tool outputs and conversation messages
//! (#951/#1046). Split from `agent_loop.rs` to respect the 750-line cap.

use super::*;
use crate::domain::session::SpillEntry;

/// Args for building a tool result message (avoids clippy 5-arg limit).
pub(super) struct ToolMessageArgs<'a> {
    pub(super) tc: &'a ToolCall,
    pub(super) content: String,
    pub(super) image_blocks: Vec<crate::domain::tool::ImageBlock>,
    pub(super) spill_id: String,
    pub(super) is_error: bool,
}

impl AgentLoopImpl {
    pub(super) fn build_tool_message(&self, args: ToolMessageArgs) -> Message {
        let mut tool_msg = Message::tool(args.tc.id.clone(), args.content);
        tool_msg.tool_name = Some(args.tc.name.clone());
        tool_msg.input_preview =
            Some(context_pruning::truncate_utf8_safe(&args.tc.arguments, 100).into_owned());
        tool_msg.spill_id = Some(args.spill_id);
        tool_msg.image_blocks = args.image_blocks;
        tool_msg.invalidate_token_cache();
        tool_msg.is_error = args.is_error;
        tool_msg
    }

    // NOTE: tool-output spilling has no ephemeral-session (empty key) guard —
    // deliberately. Tool-result collapse (#1017) can fire within a single
    // ephemeral run and its recall() stubs must stay resolvable, so ephemeral
    // tool spills persist under the sanitized empty-key store path. Guarding
    // here would break recall of collapsed tool output in `--no-session`
    // runs. Conversation-message spilling is symmetric since PR #1048 (its
    // former empty-key guard let collapse/ladder stub or drop content that
    // was never written to disk); both symmetry sides are pinned by
    // `ephemeral_session_spills_both_tool_output_and_conversation_messages`.
    pub(super) async fn spill_tool_message(&self, tool_msg: &mut Message) {
        let Some(ref spill_store) = self.spill_store else {
            return;
        };

        // Move (not clone) the content into the SpillEntry for the borrowing
        // append, then move it back — avoids copying up to 1MB of tool output.
        let content = std::mem::take(&mut tool_msg.content);
        let entry = SpillEntry {
            id: tool_msg.spill_id.clone().unwrap_or_default(),
            tool: tool_msg
                .tool_name
                .clone()
                .unwrap_or_else(|| "tool".to_string()),
            input_preview: tool_msg.input_preview.clone().unwrap_or_default(),
            tokens: context_pruning::estimate_tokens(&content),
            content,
        };
        if let Err(e) = spill_store.append(&self.session_key, &entry).await {
            tracing::warn!(target: "context_prune", error = %e, "failed to spill tool output");
        }
        // Restore content back into the message (entry is consumed here).
        tool_msg.content = entry.content;
        tool_msg.invalidate_token_cache();
    }

    /// Spill a conversation (assistant/user) message at creation so it is
    /// immediately recallable (#1046 AC1). No-op without a spill store; the
    /// ephemeral-session (empty key) guard lives in the shared writer.
    pub(super) async fn spill_conversation_message(&self, msg: &mut Message) {
        if let Some(ref spill_store) = self.spill_store {
            context_pruning::messages::spill_conversation_message(
                msg,
                spill_store.as_ref(),
                &self.session_key,
            )
            .await;
        }
    }
}
