use super::*;

use crate::shell::socket_path::usable_socket_path;

impl App {
    pub(super) fn ensure_synced_subagent_feed(&mut self, id: &str) {
        if let Some(feed) = self.ac().roster.feeds.get(id) {
            // Upgrade a stale inspection-only feed once the child's socket has
            // become usable (#1466 round 2): restored sub-agents are focused
            // before their live socket is known, and the early-return here
            // used to pin them inspection-only forever, so user sends failed
            // "unattached" while master-driven messaging worked.
            // Cheap flag first (PR #1485 review): a direct feed never needs the
            // socket probe (symlink_metadata + canonicalize), which would
            // otherwise run on every roster event for every warm sub-agent.
            if !feed.inspection_only {
                return;
            }
            let socket_now_usable = self
                .ac()
                .roster
                .tracked
                .get(id)
                .is_some_and(|t| usable_socket_path(t.info.socket_path.as_deref()));
            if !socket_now_usable {
                return;
            }
            if let Some(stale) = self.ac_mut().roster.feeds.remove(id) {
                stale.handle.abort();
            }
        }
        self.open_subagent_feed(id, crate::agents::feed::FeedAuthority::WarmSync);
    }

    /// Whether `id` has a direct child-socket feed (not inspection-only) —
    /// the precondition for delivering Prompt/FollowUp commands.
    pub(super) fn subagent_feed_is_direct(&self, id: &str) -> bool {
        self.ac()
            .roster
            .feeds
            .get(id)
            .is_some_and(|feed| feed.is_direct())
    }

    /// The child `id`'s session stats went stale: ask the child for them
    /// over its direct feed, unless a request is already outstanding (then
    /// one follow-up goes once it is answered). The reply lands in that
    /// child's footer through `update_session_footer`. An inspection feed,
    /// routed through the master, is left alone: its allowlist carries no
    /// session stats.
    pub(super) fn request_subagent_session_stats(&mut self, id: &str) {
        let now = std::time::Instant::now();
        let Some(feed) = self.ac_mut().roster.feeds.get_mut(id) else {
            return;
        };
        if feed.is_direct() && feed.stats_refresh.should_request(now) {
            Self::send_stats_request(id, feed);
        }
    }

    /// A stats reply reached the child `id`'s feed: settle the outstanding
    /// request, or send the follow-up a trigger seen meanwhile asked for.
    pub(super) fn note_subagent_stats_answered(&mut self, id: &str) {
        let now = std::time::Instant::now();
        let Some(feed) = self.ac_mut().roster.feeds.get_mut(id) else {
            return;
        };
        if feed.stats_refresh.answered(now) {
            Self::send_stats_request(id, feed);
        }
    }

    /// Most other events from the child `id` (those that reach the end of
    /// its stream routing): send a stats request the feed could not queue
    /// earlier. Nothing else happens on this per-token path.
    pub(super) fn retry_owed_subagent_stats(&mut self, id: &str) {
        let Some(feed) = self.ac_mut().roster.feeds.get_mut(id) else {
            return;
        };
        if feed.stats_refresh.owes_request()
            && feed.stats_refresh.should_request(std::time::Instant::now())
        {
            Self::send_stats_request(id, feed);
        }
    }

    /// Whether the child `id`'s own feed has reported its latest run's end
    /// (its session is not running). A child with no session view has
    /// reported nothing.
    pub(super) fn subagent_feed_saw_run_end(&self, id: &str) -> bool {
        self.ac()
            .roster
            .sessions
            .get(id)
            .is_some_and(|session| session.observed_run_state && !session.running)
    }

    fn send_stats_request(id: &str, feed: &mut FeedState) {
        let request = crate::protocol::subagent_stats::subagent_stats_request();
        if let Err(error) = feed.cmd_tx.try_send(request) {
            tracing::debug!(agent = %id, %error, "sub-agent stats request not queued");
            feed.stats_refresh.not_sent();
        }
    }

    /// Open a root-routed inspection feed for `id`. The TUI no longer consumes
    /// raw child socket paths from topology snapshots; safe inspection commands
    /// are sent to the master connection with `agent_id` and routed by the agent
    /// through the nearest reachable ancestor (#1442).
    fn open_subagent_feed(&mut self, id: &str, authority: crate::agents::feed::FeedAuthority) {
        if self.ac().roster.feeds.contains_key(id) {
            return;
        }
        let Some(tracked) = self.ac().roster.tracked.get(id) else {
            return;
        };
        let socket = tracked.info.socket_path.clone();
        let agent_id = id.to_string();
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<Command>(32);
        use tracing::instrument::WithSubscriber;
        let connect_dispatch = tracing::dispatcher::get_default(Clone::clone);
        let inspection_only = !usable_socket_path(socket.as_deref());
        // Every id this feed mints (direct-socket literals and routed
        // inspection ids alike) remains exactly attributable to this
        // connection, so broadcast responses cannot settle another client.
        let handle = if !inspection_only {
            let path = std::path::PathBuf::from(socket.expect("checked usable socket"));
            let tx = self.subagents.event_tx.clone();
            let agent_id_for_task = agent_id.clone();
            let task = async move {
                let Ok(mut client) = Client::connect(&path).await else {
                    return;
                };
                let _ = client
                    .send(&Command::GetState {
                        id: Some("subagent-state".into()),
                        agent_id: None,
                    })
                    .await;
                let _ = client
                    .send(&Command::Sync {
                        id: Some("subagent-sync".into()),
                        epoch: 0,
                        since_rev: 0,
                        agent_id: None,
                    })
                    .await;
                // Ask for the child's own stats (cost, cache hit): an idle
                // child pushes no stats on connect, and a busy child's
                // snapshot predates its run's usage. A busy child answers
                // once its prompt ends.
                let _ = client
                    .send(&crate::protocol::subagent_stats::subagent_stats_request())
                    .await;
                use crate::shell::connection::SourcedEvent;
                loop {
                    tokio::select! {
                        ev = client.recv() => match ev {
                            Some(ev) => if tx.send(SourcedEvent::Subagent(agent_id_for_task.clone(), ev)).await.is_err() { break; },
                            None => break,
                        },
                        cmd = cmd_rx.recv() => match cmd {
                            Some(cmd) => { let _ = client.send(&cmd).await; }
                            None => break,
                        },
                    }
                }
            };
            tokio::spawn(task.with_subscriber(connect_dispatch))
        } else {
            let root_sender = self.ac().transport.clone_sender();
            let task = async move {
                let _ = root_sender.try_send(
                    &Command::GetState {
                        id: Some("initial".into()),
                        agent_id: None,
                    }
                    .with_inspection_agent_id(&agent_id)
                    .expect("get_state is routable inspection"),
                );
                let _ = root_sender.try_send(
                    &Command::Sync {
                        id: Some("initial".into()),
                        epoch: 0,
                        since_rev: 0,
                        agent_id: None,
                    }
                    .with_inspection_agent_id(&agent_id)
                    .expect("sync is routable inspection"),
                );
                // A cold routed feed has no direct child stream to backfill from.
                // Request the child's newest transcript page explicitly so an
                // already-idle nested container child renders when focused even
                // if sync has no new delta to project.
                let _ = root_sender.try_send(
                    &Command::GetMessagesTail {
                        id: Some("initial".into()),
                        count: 20,
                        agent_id: None,
                    }
                    .with_inspection_agent_id(&agent_id)
                    .expect("get_messages_tail is routable inspection"),
                );
                while let Some(cmd) = cmd_rx.recv().await {
                    if let Some(routed) = cmd.with_inspection_agent_id(&agent_id) {
                        let _ = root_sender.try_send(&routed);
                    }
                }
            };
            tokio::spawn(task.with_subscriber(connect_dispatch))
        };
        self.ac_mut().roster.feeds.insert(
            id.to_string(),
            FeedState::from_parts(
                crate::agents::runtime::FeedRuntime {
                    cmd_tx,
                    handle,
                    inspection_only,
                },
                Self::feed_sync_state(authority, !inspection_only),
            ),
        );
    }

    /// A new feed's sync state. A direct feed has just sent its connect-time
    /// stats request, so that request is outstanding.
    fn feed_sync_state(
        authority: crate::agents::feed::FeedAuthority,
        direct: bool,
    ) -> crate::agents::feed::FeedSyncState {
        let mut sync = crate::agents::feed::FeedSyncState::new(authority);
        if direct {
            sync.stats_refresh =
                crate::agents::feed::StatsRefresh::sent_at(std::time::Instant::now());
        }
        sync
    }
}
