use super::*;

impl App {
    pub(super) fn note_ledger_advanced(&mut self, agent_id: &str, epoch: u64, rev: u64) {
        if let Some(feed) = self.subagents.feeds.get_mut(agent_id) {
            feed.epoch = epoch;
            feed.rev = rev;
            feed.last_fresh_at = Some(std::time::Instant::now());
        }
    }

    pub(super) fn route_sync_response(&mut self, agent_id: &str, data: &serde_json::Value) {
        let Ok(delta) =
            serde_json::from_value::<crate::interface::ledger_sync::SyncDelta>(data.clone())
        else {
            return;
        };
        if let Some(feed) = self.subagents.feeds.get_mut(agent_id) {
            let entries = feed.transcript.apply_sync_delta(&delta);
            feed.epoch = delta.epoch;
            feed.rev = delta.rev;
            feed.last_fresh_at = Some(std::time::Instant::now());
            if let Some(session) = self.subagents.sessions.get_mut(agent_id) {
                session.chat.clear();
                for entry in entries {
                    session.chat.add_entry(entry);
                }
            }
        }
    }

    pub(super) fn note_sync_capability(&mut self, agent_id: &str, data: &serde_json::Value) {
        if let Some(feed) = self.subagents.feeds.get_mut(agent_id) {
            feed.supports_sync = crate::interface::ledger_sync::supports_sync(data);
            feed.last_fresh_at = Some(std::time::Instant::now());
        }
    }
}
