use super::*;

use crate::shell::socket_path::usable_socket_path;

impl App {
    pub(super) fn ensure_synced_subagent_feed(&mut self, id: &str) {
        if self.subagents.feeds.contains_key(id) {
            return;
        }
        self.open_subagent_feed(id, crate::agents::feed::FeedAuthority::WarmSync);
    }

    /// Open a root-routed inspection feed for `id`. The TUI no longer consumes
    /// raw child socket paths from topology snapshots; safe inspection commands
    /// are sent to the master connection with `agent_id` and routed by the agent
    /// through the nearest reachable ancestor (#1442).
    fn open_subagent_feed(&mut self, id: &str, authority: crate::agents::feed::FeedAuthority) {
        if self.subagents.feeds.contains_key(id) {
            return;
        }
        let Some(tracked) = self.subagents.tracked.get(id) else {
            return;
        };
        let socket = tracked.info.socket_path.clone();
        let agent_id = id.to_string();
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<Command>(32);
        use tracing::instrument::WithSubscriber;
        let connect_dispatch = tracing::dispatcher::get_default(Clone::clone);
        let inspection_only = !usable_socket_path(socket.as_deref());
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
                use crate::shell::connection::{Source, TabId};
                loop {
                    tokio::select! {
                        ev = client.recv() => match ev {
                            Some(ev) => if tx.send((Source::Subagent(TabId::MASTER, agent_id_for_task.clone()), Some(ev))).await.is_err() { break; },
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
            let root_sender = self.connection.clone_sender();
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
        self.subagents.feeds.insert(
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
