use super::RecoverMessage;
use crate::application::sessions::active_session::{ActiveSessionHandle, ActiveSessionState};
use crate::application::sessions::dto::{
    ContentSelector, RecoveredContent, RecoveryError, RecoveryRequest,
};
use crate::application::sessions::ports::ContextSpillStore;
use crate::domain::error::DomainError;
use crate::domain::ids::MessageId;
use crate::domain::message::{Message, ToolCall};
use crate::domain::session::{SpillEntries, SpillEntry, SpillIndex};
use crate::domain::session_identity::{SessionIdentity, SpillId};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

type Recall<'a> =
    Pin<Box<dyn Future<Output = Result<Option<SpillEntry>, DomainError>> + Send + 'a>>;
type Unit<'a> = Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + 'a>>;

/// A retention store with nothing in it.
#[derive(Debug)]
pub(crate) struct NoopSpillStore;

impl ContextSpillStore for NoopSpillStore {
    fn append(&self, _: &SessionIdentity, _: &SpillEntry) -> Unit<'_> {
        Box::pin(async { Ok(()) })
    }
    fn recall(&self, _: &SessionIdentity, _: &SpillId) -> Recall<'_> {
        Box::pin(async { Ok(None) })
    }
    fn list_entries(
        &self,
        _: &SessionIdentity,
    ) -> Pin<Box<dyn Future<Output = Result<SpillEntries, DomainError>> + Send + '_>> {
        Box::pin(async { Ok(Arc::new(Vec::new())) })
    }
    fn clear(&self, _: &SessionIdentity) -> Unit<'_> {
        Box::pin(async { Ok(()) })
    }
}

/// A session-aware in-memory retention store recording every recall.
#[derive(Debug, Default)]
pub(crate) struct MemSpillStore {
    entries: Mutex<Vec<(String, SpillEntry)>>,
    recalled: Mutex<Vec<(String, String)>>,
    fail: bool,
}

impl MemSpillStore {
    pub(crate) fn with_session_entry(session_key: &str, entry: SpillEntry) -> Self {
        Self {
            entries: Mutex::new(vec![(session_key.to_string(), entry)]),
            ..Self::default()
        }
    }
    pub(crate) fn with_recall_error() -> Self {
        Self {
            fail: true,
            ..Self::default()
        }
    }
    pub(crate) fn recalled(&self) -> Vec<(String, String)> {
        self.recalled.lock().unwrap().clone()
    }
}

impl ContextSpillStore for MemSpillStore {
    fn append(&self, _: &SessionIdentity, _: &SpillEntry) -> Unit<'_> {
        Box::pin(async { Ok(()) })
    }
    fn recall(&self, identity: &SessionIdentity, id: &SpillId) -> Recall<'_> {
        let key = identity.runtime_key().to_string();
        let id = id.as_str().to_string();
        self.recalled
            .lock()
            .unwrap()
            .push((key.clone(), id.clone()));
        let found = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .find(|(k, e)| *k == key && e.id == id)
            .map(|(_, e)| e.clone());
        let fail = self.fail;
        Box::pin(async move {
            if fail {
                Err(DomainError::Tool("recall failed".into()))
            } else {
                Ok(found)
            }
        })
    }
    fn list_entries(
        &self,
        _: &SessionIdentity,
    ) -> Pin<Box<dyn Future<Output = Result<SpillEntries, DomainError>> + Send + '_>> {
        let index: Vec<SpillIndex> = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .map(|(_, e)| SpillIndex {
                id: e.id.clone(),
                tool: e.tool.clone(),
                input_preview: e.input_preview.clone(),
                tokens: e.tokens,
            })
            .collect();
        Box::pin(async move { Ok(Arc::new(index)) })
    }
    fn clear(&self, _: &SessionIdentity) -> Unit<'_> {
        Box::pin(async { Ok(()) })
    }
}

pub(crate) fn spill_entry(id: &str, content: &str) -> SpillEntry {
    SpillEntry {
        id: id.into(),
        tool: "message".into(),
        input_preview: String::new(),
        tokens: 1,
        content: content.into(),
    }
}

pub(crate) fn collapsed_message(spill_id: &str) -> Message {
    let mut message = Message::assistant(format!("recall(\"{spill_id}\")"), vec![]);
    message.is_collapsed = true;
    message.spill_id = Some(spill_id.into());
    message
}

fn handle(key: &str, store: Option<Arc<dyn ContextSpillStore>>) -> ActiveSessionHandle {
    let mut state = ActiveSessionState::new(SessionIdentity::from_persisted_key(key));
    state.set_spill_store(store);
    Arc::new(tokio::sync::RwLock::new(state))
}

fn whole(id: &str) -> RecoveryRequest {
    RecoveryRequest {
        message_id: MessageId::from(id),
        selector: ContentSelector::whole_message(),
    }
}

#[tokio::test]
async fn resolves_the_ledger_full_copy_before_the_collapsed_live_entry() {
    let full = Message::assistant("the full answer", vec![]);
    let id = full.id().to_string();
    let handle = handle("", None);
    handle
        .write()
        .await
        .record_full(std::slice::from_ref(&full));
    let mut stub = full.clone();
    stub.content = "recall(spilled)".into();
    stub.is_collapsed = true;
    handle.write().await.publish(&[stub]);
    let recover = RecoverMessage::new(handle);
    let RecoveredContent::Message {
        message,
        range,
        thinking_offset,
        ranged,
    } = recover.execute(&whole(&id), &[]).await.unwrap()
    else {
        panic!("whole message");
    };
    assert_eq!(message.content, "the full answer");
    assert_eq!((range.start, range.end), (0, "the full answer".len()));
    assert_eq!(thinking_offset, 0);
    assert!(!ranged);
}

#[tokio::test]
async fn recalls_a_collapsed_stub_through_the_session_identity_and_falls_back_to_the_stub() {
    let stub = collapsed_message("spill-1");
    let id = stub.id().to_string();
    let store = Arc::new(MemSpillStore::with_session_entry(
        "cli:resumed",
        spill_entry("spill-1", "resumed session content"),
    ));
    let handle = handle("cli:resumed", Some(store.clone()));
    handle.write().await.publish(std::slice::from_ref(&stub));
    let recover = RecoverMessage::new(handle.clone());
    let resolved = recover
        .resolve(&MessageId::from(id.as_str()))
        .await
        .unwrap();
    assert_eq!(resolved.content, "resumed session content");
    assert_eq!(
        store.recalled(),
        vec![("cli:resumed".to_string(), "spill-1".to_string())],
        "the recall is keyed by the active session's identity"
    );

    // Missing entry: the live stub is returned unchanged.
    handle
        .write()
        .await
        .set_spill_store(Some(Arc::new(MemSpillStore::default())));
    let resolved = recover
        .resolve(&MessageId::from(id.as_str()))
        .await
        .unwrap();
    assert!(resolved.is_collapsed);
    assert_eq!(resolved.spill_id.as_deref(), Some("spill-1"));

    // Store error: the same stub fallback.
    handle
        .write()
        .await
        .set_spill_store(Some(Arc::new(MemSpillStore::with_recall_error())));
    let resolved = recover
        .resolve(&MessageId::from(id.as_str()))
        .await
        .unwrap();
    assert!(resolved.is_collapsed);
}

#[tokio::test]
async fn unknown_refs_consult_the_fallback_then_fail_structurally() {
    let handle = handle("", None);
    let recover = RecoverMessage::new(handle);
    let live_only = Message::user("only in the loop's vector");
    let id = live_only.id().to_string();
    let RecoveredContent::Message { message, .. } = recover
        .execute(&whole(&id), std::slice::from_ref(&live_only))
        .await
        .unwrap()
    else {
        panic!("whole message");
    };
    assert_eq!(message.content, "only in the loop's vector");
    for missing in ["00000000-0000-0000-0000-000000000000", "not-a-uuid"] {
        let err = recover
            .execute(&whole(missing), std::slice::from_ref(&live_only))
            .await
            .unwrap_err();
        assert_eq!(
            err,
            RecoveryError::MessageNotFound(MessageId::from(missing))
        );
        assert_eq!(err.to_string(), format!("message not found: {missing}"));
        assert_eq!(err.message_id().as_str(), missing);
    }
}

#[test]
fn select_maps_ranges_thinking_offsets_and_tool_calls() {
    let mut message = Message::assistant(
        "aé日z",
        vec![ToolCall {
            id: "call-1".into(),
            name: "bash".into(),
            arguments: "{\"command\":\"echo\"}".into(),
        }],
    );
    message
        .thinking_blocks
        .push(crate::domain::message::ThinkingBlock::Normal {
            thinking: "why".into(),
            signature: "sig".into(),
        });
    let RecoveredContent::Message {
        range,
        thinking_offset,
        ranged,
        ..
    } = RecoverMessage::select(
        message.clone(),
        &ContentSelector::Message {
            offset: Some(2),
            thinking_offset: None,
            limit: Some(4),
        },
    )
    .unwrap()
    else {
        panic!("message");
    };
    // Offset 2 sits inside `é`: back to byte 1; 4 bytes forward lands inside
    // `日` (bytes 3..6): back to 3.
    assert_eq!((range.start, range.end), (1, 3));
    assert_eq!(thinking_offset, 2, "defaults to the content offset");
    assert!(ranged);

    let RecoveredContent::Message {
        thinking_offset, ..
    } = RecoverMessage::select(
        message.clone(),
        &ContentSelector::Message {
            offset: None,
            thinking_offset: Some(7),
            limit: None,
        },
    )
    .unwrap()
    else {
        panic!("message");
    };
    assert_eq!(thinking_offset, 7);

    let RecoveredContent::ToolCallArguments {
        message_id,
        tool_call,
        range,
    } = RecoverMessage::select(
        message.clone(),
        &ContentSelector::ToolCallArguments {
            tool_call_id: "call-1".into(),
            offset: Some(2),
            limit: Some(4),
        },
    )
    .unwrap()
    else {
        panic!("tool call");
    };
    assert_eq!(message_id.as_str(), message.id().to_string());
    assert_eq!(tool_call.name, "bash");
    assert_eq!((range.start, range.end), (2, 6));

    let err = RecoverMessage::select(
        message.clone(),
        &ContentSelector::ToolCallArguments {
            tool_call_id: "missing".into(),
            offset: None,
            limit: None,
        },
    )
    .unwrap_err();
    assert!(matches!(err, RecoveryError::ToolCallNotFound { .. }));
    assert!(err.to_string().contains("missing"));
    assert_eq!(err.message_id().as_str(), message.id().to_string());
}

/// A retention store whose recall blocks until released, so a lifecycle
/// replacement can be interposed while the lock is not held.
#[derive(Debug)]
struct BlockingSpillStore {
    entry: SpillEntry,
    started: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    release: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}

impl ContextSpillStore for BlockingSpillStore {
    fn append(&self, _: &SessionIdentity, _: &SpillEntry) -> Unit<'_> {
        Box::pin(async { Ok(()) })
    }
    fn recall(&self, _: &SessionIdentity, _: &SpillId) -> Recall<'_> {
        let started = self.started.lock().unwrap().take();
        let release = self.release.lock().unwrap().take();
        let entry = self.entry.clone();
        Box::pin(async move {
            if let Some(started) = started {
                let _ = started.send(());
            }
            if let Some(release) = release {
                let _ = release.await;
            }
            Ok(Some(entry))
        })
    }
    fn list_entries(
        &self,
        _: &SessionIdentity,
    ) -> Pin<Box<dyn Future<Output = Result<SpillEntries, DomainError>> + Send + '_>> {
        Box::pin(async { Ok(Arc::new(Vec::new())) })
    }
    fn clear(&self, _: &SessionIdentity) -> Unit<'_> {
        Box::pin(async { Ok(()) })
    }
}

/// Deferred recall must not hold the read lock, and a result captured
/// before a lifecycle replacement must be discarded rather than leaking old
/// session content into the response.
#[tokio::test]
async fn spill_recall_retries_after_concurrent_history_replacement() {
    let old = collapsed_message("same-spill");
    let message_id = MessageId::from(old.id().to_string());
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let old_store = Arc::new(BlockingSpillStore {
        entry: spill_entry("same-spill", "old secret"),
        started: Mutex::new(Some(started_tx)),
        release: Mutex::new(Some(release_rx)),
    });
    let handle = handle("cli:old", Some(old_store));
    handle.write().await.publish(std::slice::from_ref(&old));
    let recover = Arc::new(RecoverMessage::new(handle.clone()));

    let resolving = tokio::spawn({
        let recover = recover.clone();
        let message_id = message_id.clone();
        async move { recover.resolve(&message_id).await }
    });
    started_rx.await.expect("old spill recall starts");

    // A write lock is obtainable while recall is blocked: no guard is held
    // across store I/O. Replace the complete history + identity.
    tokio::time::timeout(Duration::from_millis(250), async {
        let mut state = handle.write().await;
        state.clear();
        state.switch_identity(SessionIdentity::from_persisted_key("cli:new"), None);
    })
    .await
    .expect("history replacement must not wait for spill I/O");
    release_tx.send(()).expect("release old spill read");

    assert!(
        resolving.await.unwrap().is_none(),
        "old-session recall must be discarded and retried against new history"
    );
}

#[test]
fn debug_output_is_opaque() {
    let recover = RecoverMessage::new(handle("", None));
    assert!(format!("{recover:?}").starts_with("RecoverMessage"));
}
