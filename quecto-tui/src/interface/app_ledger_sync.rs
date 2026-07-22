use super::*;

#[cfg(test)]
#[path = "app_ledger_sync_tests.rs"]
mod app_ledger_sync_tests;

impl App {
    pub(super) fn note_ledger_advanced(&mut self, agent_id: &str, epoch: u64, rev: u64) {
        if let Some(feed) = self.subagents.feeds.get_mut(agent_id) {
            let since_rev = if feed.epoch == epoch { feed.rev } else { 0 };
            if feed.supports_sync && rev > since_rev {
                let _ = feed.cmd_tx.try_send(Command::Sync {
                    id: Some("subagent-sync".into()),
                    epoch,
                    since_rev,
                });
            }
            feed.epoch = epoch;
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
            if feed.epoch != 0 && delta.epoch != feed.epoch {
                return;
            }
            let entries = feed.transcript.apply_sync_delta(&delta);
            feed.epoch = delta.epoch;
            feed.rev = delta.rev;
            feed.last_fresh_at = Some(std::time::Instant::now());
            if !delta.caught_up {
                if let Some(next_rev) = delta.next_rev {
                    let _ = feed.cmd_tx.try_send(Command::Sync {
                        id: Some("subagent-sync".into()),
                        epoch: delta.epoch,
                        since_rev: next_rev,
                    });
                }
            }
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
