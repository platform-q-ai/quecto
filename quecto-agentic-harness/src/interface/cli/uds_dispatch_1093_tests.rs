//! Regression tests for #1093: `get_message` resolves collapsed spill refs.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::message::Message;
use crate::domain::session::{ContextSpillStore, Session, SessionStore, SpillEntry, SpillIndex};
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::interface::cli::protocol::AgentCommand;
use crate::interface::cli::uds::{DispatchCtx, dispatch_command};
use crate::interface::cli::uds_cancel::{CancelHandle, CancelSlot};
use crate::interface::cli::uds_ext_protocol::new_client_tool_registry;
use crate::interface::cli::uds_session::{AgentSession, compute_session_stats};

#[derive(Debug, Default)]
struct MemSpillStore {
    entries: Mutex<HashMap<(String, String), SpillEntry>>,
    recalls: Mutex<Vec<(String, String)>>,
    recall_error: bool,
}

impl MemSpillStore {
    fn with_entry(entry: SpillEntry) -> Self {
        Self::with_session_entry("cli:test", entry)
    }

    fn with_session_entry(session_key: &str, entry: SpillEntry) -> Self {
        Self {
            entries: Mutex::new(HashMap::from([(
                (session_key.to_string(), entry.id.clone()),
                entry,
            )])),
            recalls: Mutex::new(Vec::new()),
            recall_error: false,
        }
    }

    fn with_recall_error() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            recalls: Mutex::new(Vec::new()),
            recall_error: true,
        }
    }

    fn recall_count(&self) -> usize {
        self.recalls.lock().unwrap().len()
    }

    fn recalled(&self) -> Vec<(String, String)> {
        self.recalls.lock().unwrap().clone()
    }
}

impl ContextSpillStore for MemSpillStore {
    fn append(
        &self,
        session_key: &str,
        entry: &SpillEntry,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<(), crate::domain::error::DomainError>>
                + Send
                + '_,
        >,
    > {
        self.entries
            .lock()
            .unwrap()
            .insert((session_key.to_string(), entry.id.clone()), entry.clone());
        Box::pin(async { Ok(()) })
    }

    fn recall(
        &self,
        session_key: &str,
        id: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Option<SpillEntry>, crate::domain::error::DomainError>,
                > + Send
                + '_,
        >,
    > {
        self.recalls
            .lock()
            .unwrap()
            .push((session_key.to_string(), id.to_string()));
        if self.recall_error {
            return Box::pin(async {
                Err(crate::domain::error::DomainError::Other("boom".into()))
            });
        }
        let hit = self
            .entries
            .lock()
            .unwrap()
            .get(&(session_key.to_string(), id.to_string()))
            .cloned();
        Box::pin(async move { Ok(hit) })
    }

    fn list_entries(
        &self,
        _session_key: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Arc<Vec<SpillIndex>>, crate::domain::error::DomainError>,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async { Ok(Arc::new(Vec::new())) })
    }

    fn clear(
        &self,
        _session_key: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<(), crate::domain::error::DomainError>>
                + Send
                + '_,
        >,
    > {
        self.entries.lock().unwrap().clear();
        Box::pin(async { Ok(()) })
    }
}

struct Fixture {
    agent: AgentLoopImpl,
    messages: Vec<Message>,
    session: AgentSession,
    session_key: String,
    store: FileSessionStore,
    _tmp: tempfile::TempDir,
    cancel: CancelHandle,
}

impl Fixture {
    fn new(spill_store: Option<Arc<dyn ContextSpillStore>>) -> Self {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = FileSessionStore::new(tmp.path());
        Self {
            agent: AgentLoopImpl::new(AgentLoopConfig {
                provider: crate::interface::test_support::make_stub_provider(),
                tool_registry: Box::new(
                    crate::infrastructure::tools::registry::ToolRegistryImpl::new(),
                ),
                model: "stub".into(),
                max_tokens: 100,
                temperature: 0.0,
                spill_store,
                session_key: "cli:test".into(),
                context_collapse_after_tool_calls: u32::MAX,
                max_context_tokens: 190_000,
                progress_callback: None,
                streaming: false,
                effort: None,
                system_prompt_provider: None,
                audit_log: None,
                pin_recent_turns: 2,
                context_collapse_after_messages: u32::MAX,
                model_context_window: None,
            }),
            messages: Vec::new(),
            session: AgentSession::new("stub".into(), "cli:test".into()),
            session_key: "cli:test".into(),
            store,
            _tmp: tmp,
            cancel: Arc::new(Mutex::new(CancelSlot::Idle)),
        }
    }

    fn ctx(
        &mut self,
        broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    ) -> DispatchCtx<'_> {
        let initial_stats = compute_session_stats(&self.session_key, &self.messages);
        let snapshot_messages = self.messages.clone();
        let spill_store = self.agent.spill_store().cloned();
        let mut snapshot_data =
            crate::interface::cli::uds_snapshots::ConversationSnapshotData::from_messages(
                snapshot_messages,
            );
        snapshot_data.set_spill_store(spill_store, self.session_key.clone());
        DispatchCtx {
            wire_mode: crate::interface::cli::uds_wire::ConnectionWireMode::legacy(),
            base_dir: self._tmp.path(),
            agent: &mut self.agent,
            messages: &mut self.messages,
            conversation_snapshot: Arc::new(tokio::sync::RwLock::new(snapshot_data)),
            state_snapshot: Arc::new(tokio::sync::RwLock::new(
                self.session.state_snapshot(0, None, 0, None),
            )),
            session_stats_snapshot: Arc::new(tokio::sync::RwLock::new(initial_stats)),
            extension_snapshot: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            busy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            session: &mut self.session,
            stdout: None,
            session_key: &mut self.session_key,
            session_store: &self.store,
            ephemeral: false,
            system_prompt: "",
            cancel_handle: self.cancel.clone(),
            turn_control: Arc::default(),
            broadcast_tx,
            ext_registry: None,
            client_tool_registry: new_client_tool_registry(),
            current_client_id: 0,
            subagent_registry: None,
            notification_rx: None,
            workflow_state: None,
            workflow_config: None,
            provider_reload: None,
            provider_reload_inputs: None,
            last_persisted_message_index: 0,
            durable_prefix_dirty: false,
        }
    }
}

fn spill_entry(id: &str, content: &str) -> SpillEntry {
    SpillEntry {
        id: id.into(),
        tool: "message".into(),
        input_preview: String::new(),
        tokens: 4,
        content: content.into(),
    }
}

fn collapsed_message(spill_id: &str) -> Message {
    let mut msg = Message::assistant(format!("[collapsed] recall(\"{spill_id}\")"), vec![]);
    msg.is_collapsed = true;
    msg.spill_id = Some(spill_id.into());
    msg
}

async fn next_response(rx: &mut tokio::sync::broadcast::Receiver<String>) -> serde_json::Value {
    let line = rx.recv().await.expect("dispatch emits a response");
    serde_json::from_str(&line).expect("response is json")
}

#[tokio::test]
async fn get_message_idle_recalls_full_content_for_collapsed_live_message() {
    let spill_id = "turn1:msg:assistant";
    let full = "full assistant body from spill";
    let store = Arc::new(MemSpillStore::with_entry(spill_entry(spill_id, full)));
    let mut fx = Fixture::new(Some(store.clone()));
    let collapsed = collapsed_message(spill_id);
    let message_id = collapsed.id().to_string();
    fx.messages.push(collapsed);
    let (tx, mut rx) = tokio::sync::broadcast::channel(8);

    let cmd = AgentCommand::GetMessage {
        id: Some("gm-collapsed".into()),
        message_id,
        agent_id: None,
    };

    assert!(!dispatch_command(cmd, &mut fx.ctx(Some(tx))).await);

    let response = next_response(&mut rx).await;
    assert_eq!(
        response["success"], true,
        "get_message should succeed: {response}"
    );
    assert_eq!(response["data"]["content"], full);
    assert!(
        !response["data"]["content"]
            .as_str()
            .unwrap()
            .contains("recall("),
        "full recall content should replace the collapsed recall stub: {response}"
    );
    assert_eq!(
        store.recall_count(),
        1,
        "spill store should be consulted once"
    );
}

#[tokio::test]
async fn get_message_busy_recalls_full_content_for_collapsed_snapshot_message() {
    let spill_id = "turn1:msg:assistant";
    let full = "busy resolver full content from spill";
    let collapsed = collapsed_message(spill_id);
    let message_id = collapsed.id().to_string();
    let mut snapshot_data =
        crate::interface::cli::uds_snapshots::ConversationSnapshotData::from_messages(vec![
            collapsed,
        ]);
    let spill_store = Arc::new(MemSpillStore::with_entry(spill_entry(spill_id, full)));
    snapshot_data.set_spill_store(Some(spill_store.clone()), "cli:test".into());
    let snapshot = Arc::new(tokio::sync::RwLock::new(snapshot_data));
    let registry = new_client_tool_registry();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    crate::interface::cli::uds_ext_protocol::register_client_writer(&registry, 7, tx);

    crate::interface::cli::uds_busy_get_message::service(
        (Some("busy-collapsed".into()), message_id.clone()),
        &snapshot,
        &registry,
        7,
    )
    .await;

    let line = rx.recv().await.expect("busy resolver emits a response");
    let response: serde_json::Value = serde_json::from_str(&line).expect("response is json");
    assert_eq!(
        response["success"], true,
        "get_message should succeed: {response}"
    );
    assert_eq!(response["data"]["content"], full);
    assert!(
        !response["data"]["content"]
            .as_str()
            .unwrap()
            .contains("recall("),
        "full recall content should replace the collapsed recall stub: {response}"
    );
    assert_eq!(
        spill_store.recall_count(),
        1,
        "first lookup should consult spill store"
    );

    crate::interface::cli::uds_busy_get_message::service(
        (Some("busy-collapsed-cached".into()), message_id),
        &snapshot,
        &registry,
        7,
    )
    .await;
    let line = rx
        .recv()
        .await
        .expect("cached busy resolver emits a response");
    let response: serde_json::Value = serde_json::from_str(&line).expect("response is json");
    assert_eq!(response["data"]["content"], full);
    assert_eq!(
        spill_store.recall_count(),
        2,
        "each lookup should consult the authoritative session-scoped store"
    );
}

async fn assert_idle_stub_fallback(spill_id: &str, store: Arc<MemSpillStore>) {
    let mut fx = Fixture::new(Some(store.clone()));
    let collapsed = collapsed_message(spill_id);
    let stub = collapsed.content.clone();
    let message_id = collapsed.id().to_string();
    fx.messages.push(collapsed);
    let (tx, mut rx) = tokio::sync::broadcast::channel(8);

    let cmd = AgentCommand::GetMessage {
        id: Some("gm-collapsed".into()),
        message_id,
        agent_id: None,
    };

    assert!(!dispatch_command(cmd, &mut fx.ctx(Some(tx))).await);

    let response = next_response(&mut rx).await;
    assert_eq!(
        response["success"], true,
        "live collapsed message should still resolve"
    );
    assert_eq!(response["data"]["content"], stub);
    assert!(
        response["data"]["content"]
            .as_str()
            .unwrap()
            .contains("recall(")
    );
    assert_eq!(
        store.recall_count(),
        1,
        "fallback should happen after one recall attempt"
    );
}

#[tokio::test]
async fn get_message_uses_the_snapshot_session_key_for_spill_recall() {
    let spill_id = "turn1:msg:assistant";
    let store = Arc::new(MemSpillStore::with_session_entry(
        "cli:resumed",
        spill_entry(spill_id, "resumed session content"),
    ));
    let collapsed = collapsed_message(spill_id);
    let message_id = collapsed.id().to_string();
    let mut snapshot_data =
        crate::interface::cli::uds_snapshots::ConversationSnapshotData::default();
    snapshot_data.reset_to_with_spill_store(
        std::slice::from_ref(&collapsed),
        Some(store.clone()),
        "cli:resumed".into(),
    );
    let snapshot = Arc::new(tokio::sync::RwLock::new(snapshot_data));

    let resolved =
        crate::interface::cli::uds_snapshots::resolve_get_message(&snapshot, &message_id)
            .await
            .expect("collapsed message resolves");

    assert_eq!(resolved.content, "resumed session content");
    assert_eq!(
        store.recalled(),
        vec![("cli:resumed".into(), spill_id.into())],
        "the store fake is session-aware and must observe the resumed key"
    );
}

#[tokio::test]
async fn resume_session_atomically_switches_the_snapshot_spill_namespace() {
    let spill_id = "turn1:msg:assistant";
    let store = Arc::new(MemSpillStore::with_session_entry(
        "cli:saved",
        spill_entry(spill_id, "content from resumed session"),
    ));
    let mut fx = Fixture::new(Some(store.clone()));
    let collapsed = collapsed_message(spill_id);
    fx.store
        .save(&Session {
            key: "cli:saved".into(),
            messages: vec![collapsed],
            workflow_run: None,
        })
        .await
        .unwrap();
    let snapshot = {
        let ctx = fx.ctx(None);
        ctx.conversation_snapshot.clone()
    };

    {
        let (tx, _rx) = tokio::sync::broadcast::channel(8);
        let mut ctx = fx.ctx(Some(tx));
        ctx.conversation_snapshot = snapshot.clone();
        assert!(
            !super::handle_resume_session(
                &mut ctx,
                Some("resume"),
                "resume_session",
                "saved".into(),
            )
            .await
        );
    }

    let message_id = snapshot
        .read()
        .await
        .messages
        .iter()
        .find(|message| message.is_collapsed)
        .expect("resumed snapshot contains collapsed message")
        .id()
        .to_string();
    let resolved =
        crate::interface::cli::uds_snapshots::resolve_get_message(&snapshot, &message_id)
            .await
            .expect("resumed collapsed message resolves");
    assert_eq!(resolved.content, "content from resumed session");
    assert_eq!(
        store.recalled(),
        vec![("cli:saved".into(), spill_id.into())],
        "resume must publish the loaded history with its new key in one update"
    );
}

#[tokio::test]
async fn get_message_idle_keeps_collapsed_stub_when_spill_entry_is_missing() {
    assert_idle_stub_fallback("missing-spill-entry", Arc::new(MemSpillStore::default())).await;
}

#[tokio::test]
async fn get_message_idle_keeps_collapsed_stub_when_spill_recall_errors() {
    assert_idle_stub_fallback(
        "erroring-spill-entry",
        Arc::new(MemSpillStore::with_recall_error()),
    )
    .await;
}
