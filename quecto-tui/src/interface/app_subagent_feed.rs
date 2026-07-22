use super::app_subagents::usable_socket_path;
use super::*;

impl App {
    /// Open a direct UDS connection to `id`'s own socket and fan its live stream
    /// into the shared `subagent_event_rx`, tagged with the agent id.
    pub(super) fn open_subagent_connection(&mut self, id: &str) {
        let tracked = &self.subagents.tracked;
        let Some(socket) = tracked.get(id).and_then(|t| t.info.socket_path.clone()) else {
            return;
        };
        if !usable_socket_path(Some(&socket)) {
            return;
        }
        let tx = self.subagents.event_tx.clone();
        let agent_id = id.to_string();
        let path = std::path::PathBuf::from(socket);
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<Command>(32);
        use tracing::instrument::WithSubscriber;
        let connect_dispatch = tracing::dispatcher::get_default(Clone::clone);
        let connect_task = async move {
            let Ok(mut client) = Client::connect(&path).await else {
                return;
            };
            let _ = client
                .send(&Command::GetMessages {
                    id: Some("subagent-history".into()),
                    before: None,
                })
                .await;
            let _ = client
                .send(&Command::GetState {
                    id: Some("subagent-state".into()),
                })
                .await;
            loop {
                tokio::select! {
                    ev = client.recv() => match ev {
                        Some(ev) => if tx.send((agent_id.clone(), ev)).await.is_err() { break; },
                        None => break,
                    },
                    cmd = cmd_rx.recv() => match cmd {
                        Some(cmd) => { let _ = client.send(&cmd).await; }
                        None => break,
                    },
                }
            }
        };
        let handle = tokio::spawn(connect_task.with_subscriber(connect_dispatch));
        self.subagents.feeds.insert(
            id.to_string(),
            FeedState {
                cmd_tx,
                handle,
                epoch: 0,
                rev: 0,
                last_fresh_at: None,
                supports_sync: false,
                pending_rev: None,
                transcript: crate::interface::ledger_sync::LedgerTranscript::default(),
            },
        );
    }

    /// Abort the active sub-agent connection's forwarding task, if any.
    pub(super) fn teardown_active_connection(&mut self) {
        for (_, feed) in std::mem::take(&mut self.subagents.feeds) {
            feed.handle.abort();
        }
    }
}
