use super::*;

use crate::shell::socket_path::usable_socket_path;

impl App {
    pub(super) fn ensure_synced_subagent_feed(&mut self, id: &str) {
        if self.active_conn().roster.feeds.contains_key(id) {
            return;
        }
        self.open_subagent_feed(id, crate::agents::feed::FeedAuthority::WarmSync);
    }

    /// Open a root-routed inspection feed for `id`. The TUI no longer consumes
    /// raw child socket paths from topology snapshots; safe inspection commands
    /// are sent to the master connection with `agent_id` and routed by the agent
    /// through the nearest reachable ancestor (#1442).
    fn open_subagent_feed(&mut self, id: &str, authority: crate::agents::feed::FeedAuthority) {
        if self.active_conn().roster.feeds.contains_key(id) {
            return;
        }
        let Some(tracked) = self.active_conn().roster.tracked.get(id) else {
            return;
        };
        let socket = tracked.info.socket_path.clone();
        let agent_id = id.to_string();
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<Command>(32);
        use tracing::instrument::WithSubscriber;
        let connect_dispatch = tracing::dispatcher::get_default(Clone::clone);
        let inspection_only = !usable_socket_path(socket.as_deref());
        // Every id this feed mints (direct-socket literals and routed
        // inspection ids alike) carries the tab's connection namespace
        // (#1463) so broadcast responses can never match another tab's feed.
        let ns = self.active_conn().id_namespace();
        let handle = if !inspection_only {
            let path = std::path::PathBuf::from(socket.expect("checked usable socket"));
            let tx = self.subagents.event_tx.clone();
            let agent_id_for_task = agent_id.clone();
            // The forwarded-event tag must agree with the id namespace about
            // which tab owns this feed (#1472 r1).
            let feed_tab = self.active_conn().transport.tab();
            let task = async move {
                let Ok(mut client) = Client::connect(&path).await else {
                    return;
                };
                let _ = client
                    .send(&Command::GetState {
                        id: Some(crate::shell::connection::feed_id(&ns, "subagent-state")),
                        agent_id: None,
                    })
                    .await;
                let _ = client
                    .send(&Command::Sync {
                        id: Some(crate::shell::connection::feed_id(&ns, "subagent-sync")),
                        epoch: 0,
                        since_rev: 0,
                        agent_id: None,
                    })
                    .await;
                use crate::shell::connection::SourcedEvent;
                loop {
                    tokio::select! {
                        ev = client.recv() => match ev {
                            Some(ev) => if tx.send(SourcedEvent::Subagent(feed_tab, agent_id_for_task.clone(), ev)).await.is_err() { break; },
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
            let root_sender = self.active_conn().transport.clone_sender();
            let task = async move {
                let _ = root_sender.try_send(
                    &Command::GetState {
                        id: Some("initial".into()),
                        agent_id: None,
                    }
                    .with_inspection_agent_id(&agent_id, &ns)
                    .expect("get_state is routable inspection"),
                );
                let _ = root_sender.try_send(
                    &Command::Sync {
                        id: Some("initial".into()),
                        epoch: 0,
                        since_rev: 0,
                        agent_id: None,
                    }
                    .with_inspection_agent_id(&agent_id, &ns)
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
                    .with_inspection_agent_id(&agent_id, &ns)
                    .expect("get_messages_tail is routable inspection"),
                );
                while let Some(cmd) = cmd_rx.recv().await {
                    if let Some(routed) = cmd.with_inspection_agent_id(&agent_id, &ns) {
                        let _ = root_sender.try_send(&routed);
                    }
                }
            };
            tokio::spawn(task.with_subscriber(connect_dispatch))
        };
        self.active_conn_mut().roster.feeds.insert(
            id.to_string(),
            FeedState::from_parts(
                crate::agents::runtime::FeedRuntime {
                    cmd_tx,
                    handle,
                    inspection_only,
                },
                crate::agents::feed::FeedSyncState::new(authority),
            ),
        );
    }
}
