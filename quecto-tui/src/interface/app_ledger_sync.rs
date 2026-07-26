use super::*;

#[cfg(test)]
#[path = "app_ledger_sync_tests.rs"]
mod app_ledger_sync_tests;

impl App {
    fn request_sync(feed: &mut FeedState, epoch: u64, target_rev: u64) {
        let since_rev = if feed.epoch == epoch { feed.rev } else { 0 };
        if target_rev > since_rev {
            let _ = feed.cmd_tx.try_send(Command::Sync {
                id: Some("subagent-sync".into()),
                epoch,
                since_rev,
            });
            feed.pending_rev = Some(target_rev);
        }
    }

    pub(super) fn note_ledger_advanced(&mut self, agent_id: &str, epoch: u64, rev: u64) {
        if let Some(feed) = self.subagents.feeds.get_mut(agent_id) {
            if feed.supports_sync {
                Self::request_sync(feed, epoch, rev);
            } else {
                feed.pending_rev = Some(rev);
            }
            feed.epoch = epoch;
            feed.last_fresh_at = Some(std::time::Instant::now());
        }
    }

    pub(super) fn route_sync_response(&mut self, agent_id: &str, data: &serde_json::Value) {
        let Ok(delta) = serde_json::from_value::<crate::protocol::agent_ledger_payloads::SyncDelta>(
            data.clone(),
        ) else {
            return;
        };
        if let Some(feed) = self.subagents.feeds.get_mut(agent_id) {
            if feed.epoch != 0 && delta.epoch != feed.epoch && !delta.resync {
                return;
            }
            let entries = feed.transcript.apply_sync_delta(&delta);
            feed.epoch = delta.epoch;
            feed.rev = if delta.caught_up {
                delta.rev
            } else {
                delta.next_rev.unwrap_or(feed.rev)
            };
            feed.last_fresh_at = Some(std::time::Instant::now());
            if delta.caught_up {
                feed.pending_rev = None;
            } else if let Some(next_rev) = delta.next_rev {
                let _ = feed.cmd_tx.try_send(Command::Sync {
                    id: Some("subagent-sync".into()),
                    epoch: delta.epoch,
                    since_rev: next_rev,
                });
            }
            feed.authority = crate::interface::agents::feed::FeedAuthority::SyncedAuthoritative;
            if let Some(session) = self.subagents.sessions.get_mut(agent_id) {
                session.chat.clear();
                for entry in entries {
                    session.chat.add_entry(
                        crate::interface::agents::ui::ledger_entry_to_chat_entry(entry),
                    );
                }
            }
        }
    }

    pub(super) fn note_sync_capability(&mut self, agent_id: &str, data: &serde_json::Value) {
        if let Some(feed) = self.subagents.feeds.get_mut(agent_id) {
            feed.supports_sync = crate::protocol::agent_ledger_payloads::supports_sync(data);
            if feed.supports_sync {
                if let Some(target_rev) = feed.pending_rev {
                    Self::request_sync(feed, feed.epoch, target_rev);
                }
            }
            feed.last_fresh_at = Some(std::time::Instant::now());
        }
    }
}
