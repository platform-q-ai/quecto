//! The context manager's spill writers (split from `context.rs` for its
//! decrease-only line ceiling, #2342): tool output and conversation
//! messages are retained at creation so every later stub can be recalled.
use super::ContextManager;
use crate::application::context_pruning;
use crate::domain::conversation::value_objects::message::Message;
use crate::domain::conversation::value_objects::stored_images::MessageImageRefs;
use crate::domain::sessions::entities::session::SpillEntry;

impl ContextManager {
    pub async fn spill_tool_message(&self, tool_msg: &mut Message, spill_id: String) {
        let Some(retention) = self.retention.as_ref() else {
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
            images: MessageImageRefs::of(tool_msg).into_all(),
        };
        let result = retention.retain.retain(&self.session_key, &entry).await;
        tool_msg.content = entry.content;
        tool_msg.invalidate_token_cache();
        match result {
            Ok(retained) => tool_msg.spill_id = Some(retained.id),
            Err(e) => {
                tracing::warn!(target: "context_prune", error = %e, "failed to spill tool output");
            }
        }
    }

    pub async fn spill_conversation_message(&self, msg: &mut Message) {
        if let Some(retention) = self.retention.as_ref() {
            context_pruning::messages::spill_conversation_message(
                msg,
                &retention.retain,
                &self.session_key,
            )
            .await;
        }
    }

    pub(super) async fn spill_unspilled_conversation_messages(
        &self,
        messages: &mut [Message],
    ) -> bool {
        let Some(retention) = self.retention.as_ref() else {
            return false;
        };
        let mut spilled = false;
        for msg in messages
            .iter_mut()
            .filter(|m| m.spill_id.is_none() && !m.is_manifest)
        {
            spilled |= context_pruning::messages::spill_conversation_message(
                msg,
                &retention.retain,
                &self.session_key,
            )
            .await;
        }
        spilled
    }
}
