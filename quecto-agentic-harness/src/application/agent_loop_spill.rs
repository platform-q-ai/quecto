//! Spill-writer plumbing for the agent loop: tool-result message assembly +
//! creation-time spilling of tool outputs and conversation messages
//! (#951/#1046). Split from `agent_loop.rs` to respect the 750-line cap.

use super::*;
use crate::application::context::ToolMessageBuild;

/// Args for building a tool result message (avoids clippy 5-arg limit).
pub(super) struct ToolMessageArgs<'a> {
    pub(super) tc: &'a ToolCall,
    pub(super) content: String,
    pub(super) image_blocks: Vec<crate::domain::tool::ImageBlock>,
    pub(super) is_error: bool,
}

impl AgentLoopImpl {
    pub(super) fn build_tool_message(&self, args: ToolMessageArgs) -> Message {
        self.context_manager.build_tool_message(ToolMessageBuild {
            tc: args.tc,
            content: args.content,
            image_blocks: args.image_blocks,
            is_error: args.is_error,
        })
    }

    // NOTE: tool-output spilling has no ephemeral-session (empty key) guard —
    // deliberately. Tool-result collapse (#1017) can fire within a single
    // ephemeral run and its recall() stubs must stay resolvable, so ephemeral
    // tool spills persist under the sanitized empty-key store path for the
    // duration of the run. Guarding here would break recall of collapsed tool
    // output in `--no-session` runs. Conversation-message spilling is
    // symmetric since PR #1048; both symmetry sides are pinned by
    // `ephemeral_session_spills_both_tool_output_and_conversation_messages`.
    // The privacy counterpart: ephemeral interface paths scrub the empty-key
    // spill file at run end (`FileContextSpillStore::scrub_session_spill_sync`)
    // so `--no-session` content does not outlive the run.
    //
    // `spill_id` is stamped on the message ONLY after a successful append
    // (mirroring `spill_conversation_message`): a message whose content never
    // reached disk must keep `spill_id == None` so collapse never mints an
    // unresolvable recall() stub for it.
    pub(super) async fn spill_tool_message(&self, tool_msg: &mut Message, spill_id: String) {
        self.context_manager
            .spill_tool_message(tool_msg, spill_id)
            .await;
    }

    /// Spill a conversation (assistant/user) message at creation so it is
    /// immediately recallable (#1046 AC1). No-op without a spill store. The
    /// shared writer persists for ephemeral (empty key) sessions too and
    /// stamps `spill_id` only on a successful append.
    pub(super) async fn spill_conversation_message(&self, msg: &mut Message) {
        self.context_manager.spill_conversation_message(msg).await;
    }
}
