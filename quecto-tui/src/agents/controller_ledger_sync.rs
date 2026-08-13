use super::*;

#[cfg(test)]
#[path = "app_ledger_sync_tests.rs"]
mod app_ledger_sync_tests;

impl App {
    fn request_sync(feed: &mut FeedState, ns: &str, epoch: u64, target_rev: u64) {
        let since_rev = if feed.epoch == epoch { feed.rev } else { 0 };
        if target_rev > since_rev {
            // Only mark the sync as in-flight if the send was accepted; a
            // refused send with pending_rev set would strand the feed with a
            // phantom sync that never gets answered or retried.
            if feed
                .cmd_tx
                .try_send(Command::Sync {
                    agent_id: None,
                    id: Some(crate::shell::connection::feed_id(ns, "subagent-sync")),
                    epoch,
                    since_rev,
                })
                .is_ok()
            {
                feed.pending_rev = Some(target_rev);
            }
        }
    }

    pub(super) fn note_ledger_advanced(&mut self, agent_id: &str, epoch: u64, rev: u64) {
        let ns = self.active_conn().id_namespace();
        if let Some(feed) = self.active_conn_mut().roster.feeds.get_mut(agent_id) {
            if feed.supports_sync {
                Self::request_sync(feed, &ns, epoch, rev);
            } else {
                feed.pending_rev = Some(rev);
            }
            feed.epoch = epoch;
            feed.last_fresh_at = Some(std::time::Instant::now());
        }
    }

    pub(super) fn route_sync_response(&mut self, agent_id: &str, data: &serde_json::Value) {
        let ns = self.active_conn().id_namespace();
        let Ok(delta) = serde_json::from_value::<crate::protocol::agent_ledger_payloads::SyncDelta>(
            data.clone(),
        ) else {
            return;
        };
        if let Some(feed) = self.active_conn_mut().roster.feeds.get_mut(agent_id) {
            if feed.epoch != 0 && delta.epoch != feed.epoch && !delta.resync {
                return;
            }
            let prev_rev = feed.rev;
            let prev_epoch = feed.epoch;
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
                    agent_id: None,
                    id: Some(crate::shell::connection::feed_id(&ns, "subagent-sync")),
                    epoch: delta.epoch,
                    since_rev: next_rev,
                });
            }
            feed.authority = crate::agents::feed::FeedAuthority::SyncedAuthoritative;
            // Live-tail supersession (#1259 / PR review):
            // - explicit resync, or a true epoch *change* (prev != 0), clears
            // - initial epoch latch (prev_epoch == 0) is NOT a supersede so
            //   pre-authority live tokens survive the first sync response
            // - ordinary rev advances that commit an assistant body replace the
            //   live tail (turn-end / full reconcile) — even if still "running"
            // - mid-turn rev advances that only extend committed prefix (user
            //   prompt, tool checkpoints) KEEP the live tail so tokens that
            //   raced ahead of the sync are not wiped; attach path dedupes tools
            // - rev advance after the turn goes idle clears it
            let hard_supersede = delta.resync || (prev_epoch != 0 && feed.epoch != prev_epoch);
            let rev_advanced = feed.rev != prev_rev;
            let focused = self.active_conn().roster.active_agent_id.as_deref() == Some(agent_id);
            let session_running = self
                .active_conn()
                .roster
                .sessions
                .get(agent_id)
                .is_some_and(|s| s.running);
            // Only an assistant carried by this delta can commit the current
            // live turn. Historical assistants remain in `entries` forever and
            // must not supersede a later turn's uncommitted tail.
            let delta_has_assistant = delta
                .messages
                .iter()
                .any(|message| message.role() == "assistant");
            let supersede_live =
                hard_supersede || (rev_advanced && (!session_running || delta_has_assistant));
            if let Some(session) = self.active_conn_mut().roster.sessions.get_mut(agent_id) {
                session.project_ledger_with_live(
                    entries,
                    focused && !supersede_live,
                    supersede_live,
                );
                session.reconcile_chat_retention_trim();
            }
        }
    }

    pub(super) fn note_sync_capability(&mut self, agent_id: &str, data: &serde_json::Value) {
        let ns = self.active_conn().id_namespace();
        if let Some(feed) = self.active_conn_mut().roster.feeds.get_mut(agent_id) {
            feed.supports_sync = crate::protocol::agent_ledger_payloads::supports_sync(data);
            if feed.supports_sync {
                if let Some(target_rev) = feed.pending_rev {
                    Self::request_sync(feed, &ns, feed.epoch, target_rev);
                }
            }
            feed.last_fresh_at = Some(std::time::Instant::now());
        }
    }
}
