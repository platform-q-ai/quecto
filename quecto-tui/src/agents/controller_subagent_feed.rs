use super::*;

fn usable_socket_path(path: Option<&str>) -> bool {
    path.is_some_and(|p| {
        let p = p.trim();
        let path = std::path::Path::new(p);
        if p.is_empty()
            || !path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileTypeExt;
            let Ok(metadata) = std::fs::symlink_metadata(path) else {
                return false;
            };
            metadata.file_type().is_socket() && !metadata.file_type().is_symlink()
        }
        #[cfg(not(unix))]
        {
            true
        }
    })
}

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
                loop {
                    tokio::select! {
                        ev = client.recv() => match ev {
                            Some(ev) => if tx.send((agent_id_for_task.clone(), ev)).await.is_err() { break; },
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
            let root_sender = self.client.clone_sender();
            let task = async move {
                let _ = root_sender.try_send(&Command::GetState {
                    id: Some(format!("subagent-state:{agent_id}:initial")),
                    agent_id: Some(agent_id.clone()),
                });
                let _ = root_sender.try_send(&Command::Sync {
                    id: Some(format!("subagent-sync:{agent_id}:initial")),
                    epoch: 0,
                    since_rev: 0,
                    agent_id: Some(agent_id.clone()),
                });
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
