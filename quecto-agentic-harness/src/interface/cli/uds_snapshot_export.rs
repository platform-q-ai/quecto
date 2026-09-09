use super::*;

impl ConversationSnapshotData {
    pub(in crate::interface::cli) fn export_messages(&self) -> Vec<Message> {
        let mut messages: Vec<_> = self
            .ledger_order
            .iter()
            .filter_map(|id| self.ledger.get(id).cloned())
            .collect();
        for message in &self.messages {
            if messages
                .iter()
                .all(|existing| existing.id() != message.id())
            {
                messages.push(message.clone());
            }
        }
        messages
    }
    pub(in crate::interface::cli) fn export_spill_source(
        &self,
    ) -> (Option<Arc<dyn ContextSpillStore>>, String) {
        (self.spill_store.clone(), self.spill_session_key.clone())
    }
}
