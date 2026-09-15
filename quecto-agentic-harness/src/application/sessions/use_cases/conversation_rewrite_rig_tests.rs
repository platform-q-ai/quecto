//! The rig the Clear and Rewind conversation tests drive (D6 #1975): a
//! store, a retention store and a turn-accounting fake that all write
//! into one shared journal, so the order in which a transaction touched
//! the ledger, the accounting, the retention namespace and the store is
//! observable; the store and the retention store can each be told to
//! fail.
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use super::{ClearConversation, RewindConversation, SaveSession};
use crate::application::durable_prefix::DurablePrefixLatch;
use crate::application::sessions::active_session::{ActiveSessionHandle, ActiveSessionState};
use crate::application::sessions::dto::SessionListQuery;
use crate::application::sessions::ports::session_runtime::TurnAccountingReset;
use crate::application::sessions::ports::{ContextSpillStore, SessionStore, SpillIndexList};
use crate::domain::error::DomainError;
use crate::domain::message::Message;
use crate::domain::session::{Session, SessionSummary, SpillEntries, SpillEntry};
use crate::domain::session_identity::{SessionIdentity, SpillId};
use crate::domain::workflow::WorkflowRunPersisted;

/// The effect order one transaction produced.
pub(crate) type Journal = Arc<Mutex<Vec<String>>>;

type Fut<'a, T> = Pin<Box<dyn Future<Output = Result<T, DomainError>> + Send + 'a>>;

/// A store that journals every write and records what was written.
pub(crate) struct JournalingStore {
    journal: Journal,
    fail: bool,
    pub(crate) saved: Mutex<Vec<Vec<Message>>>,
}

impl JournalingStore {
    fn write(&self, op: &str, messages: &[Message]) -> Fut<'_, ()> {
        self.journal.lock().unwrap().push(format!("store.{op}"));
        let messages = messages.to_vec();
        let fail = self.fail;
        Box::pin(async move {
            if fail {
                return Err(DomainError::Session("disk full".into()));
            }
            self.saved.lock().unwrap().push(messages);
            Ok(())
        })
    }
}

impl SessionStore for JournalingStore {
    fn load(&self, _: &SessionIdentity) -> Fut<'_, Option<Session>> {
        Box::pin(async { Ok(None) })
    }
    fn save(&self, session: &Session) -> Fut<'_, ()> {
        self.write("save", &session.messages)
    }
    fn save_delta<'a>(
        &'a self,
        _: &'a SessionIdentity,
        messages: &'a [Message],
        _: usize,
        _: Option<WorkflowRunPersisted>,
    ) -> Fut<'a, ()> {
        self.write("save_delta", messages)
    }
    fn save_clean_delta<'a>(
        &'a self,
        _: &'a SessionIdentity,
        messages: &'a [Message],
        _: usize,
        _: Option<WorkflowRunPersisted>,
    ) -> Fut<'a, ()> {
        self.write("save_clean_delta", messages)
    }
    fn exists(&self, _: &SessionIdentity) -> Fut<'_, bool> {
        Box::pin(async { Ok(false) })
    }
    fn list(&self, _: &SessionListQuery) -> Fut<'_, Vec<SessionSummary>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// A retention store that journals and records every clear.
pub(crate) struct RecordingRetention {
    journal: Journal,
    fail: bool,
    pub(crate) cleared: Mutex<Vec<String>>,
}

impl ContextSpillStore for RecordingRetention {
    fn append(&self, _: &SessionIdentity, _: &SpillEntry) -> Fut<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn recall(&self, _: &SessionIdentity, _: &SpillId) -> Fut<'_, Option<SpillEntry>> {
        Box::pin(async { Ok(None) })
    }
    fn list_entries(&self, _: &SessionIdentity) -> SpillIndexList<'_> {
        Box::pin(async { Ok(SpillEntries::default()) })
    }
    fn clear(&self, identity: &SessionIdentity) -> Fut<'_, ()> {
        self.journal.lock().unwrap().push("retention.clear".into());
        self.cleared
            .lock()
            .unwrap()
            .push(identity.runtime_key().to_string());
        let fail = self.fail;
        Box::pin(async move {
            if fail {
                Err(DomainError::Session("spill dir unwritable".into()))
            } else {
                Ok(())
            }
        })
    }
}

/// Records every reset with the visible count it was told.
pub(crate) struct RecordingAccounting {
    journal: Journal,
    pub(crate) resets: Vec<usize>,
}

impl TurnAccountingReset for RecordingAccounting {
    fn history_replaced(&mut self, visible_message_count: usize) {
        self.journal
            .lock()
            .unwrap()
            .push(format!("accounting.reset({visible_message_count})"));
        self.resets.push(visible_message_count);
    }
}

pub(crate) struct RewriteRig {
    pub(crate) state: ActiveSessionHandle,
    pub(crate) store: Arc<JournalingStore>,
    pub(crate) retention: Option<Arc<RecordingRetention>>,
    pub(crate) journal: Journal,
    pub(crate) clear: ClearConversation,
    pub(crate) rewind: RewindConversation,
}

pub(crate) struct RewriteOptions {
    pub(crate) prompt: &'static str,
    /// `Some(fail)` composes a retention store that fails when told;
    /// `None` composes none at all.
    pub(crate) retention: Option<bool>,
    pub(crate) save_fails: bool,
    pub(crate) ephemeral: bool,
}

impl Default for RewriteOptions {
    fn default() -> Self {
        Self {
            prompt: "",
            retention: Some(false),
            save_fails: false,
            ephemeral: false,
        }
    }
}

impl RewriteRig {
    pub(crate) fn accounting(&self) -> RecordingAccounting {
        RecordingAccounting {
            journal: self.journal.clone(),
            resets: Vec::new(),
        }
    }

    pub(crate) fn journal(&self) -> Vec<String> {
        self.journal.lock().unwrap().clone()
    }

    pub(crate) fn cleared_namespaces(&self) -> Vec<String> {
        self.retention
            .as_ref()
            .map(|r| r.cleared.lock().unwrap().clone())
            .unwrap_or_default()
    }

    pub(crate) fn saved(&self) -> Vec<Vec<Message>> {
        self.store.saved.lock().unwrap().clone()
    }

    pub(crate) fn watermark(&self) -> usize {
        self.state.try_read().unwrap().persisted_watermark()
    }

    pub(crate) fn set_watermark(&self, n: usize) {
        self.state.try_write().unwrap().set_persisted_watermark(n);
    }

    /// Seed the ledger with full copies of `messages`, as the loop does
    /// after a turn, so their refs resolve before the transaction.
    pub(crate) fn record(&self, messages: &[Message]) {
        self.state.try_write().unwrap().record_full(messages);
    }

    pub(crate) fn resolves(&self, message: &Message) -> bool {
        self.state
            .try_read()
            .unwrap()
            .conversation()
            .lookup(&message.id().to_string())
            .is_some()
    }

    pub(crate) fn live_contents(&self) -> Vec<String> {
        contents(
            self.state
                .try_read()
                .unwrap()
                .conversation()
                .live_messages(),
        )
    }
}

pub(crate) fn contents(messages: &[Message]) -> Vec<String> {
    messages.iter().map(|m| m.content.clone()).collect()
}

pub(crate) fn build_rewrite_rig(options: RewriteOptions) -> RewriteRig {
    let journal: Journal = Arc::default();
    let mut state = ActiveSessionState::new(SessionIdentity::from_persisted_key("cli:rewrite"));
    state.set_injected_system_prompt(options.prompt);
    let retention = options.retention.map(|fail| {
        Arc::new(RecordingRetention {
            journal: journal.clone(),
            fail,
            cleared: Mutex::new(Vec::new()),
        })
    });
    state.set_spill_store(retention.clone().map(|r| r as Arc<dyn ContextSpillStore>));
    let state: ActiveSessionHandle = Arc::new(tokio::sync::RwLock::new(state));
    let store = Arc::new(JournalingStore {
        journal: journal.clone(),
        fail: options.save_fails,
        saved: Mutex::new(Vec::new()),
    });
    let save = Arc::new(SaveSession::new(
        state.clone(),
        store.clone(),
        DurablePrefixLatch::shared(),
        None,
        None,
        options.ephemeral,
    ));
    RewriteRig {
        clear: ClearConversation::new(state.clone(), save.clone()),
        rewind: RewindConversation::new(state.clone(), save),
        state,
        store,
        retention,
        journal,
    }
}

#[tokio::test]
async fn the_fakes_journal_and_fail_as_told() {
    let rig = build_rewrite_rig(RewriteOptions {
        retention: Some(true),
        save_fails: true,
        ..RewriteOptions::default()
    });
    let identity = SessionIdentity::from_persisted_key("cli:rewrite");
    let retention = rig.retention.clone().unwrap();
    assert!(retention.clear(&identity).await.is_err());
    let entry = SpillEntry {
        id: "t1".into(),
        tool: "bash".into(),
        input_preview: String::new(),
        tokens: 1,
        content: String::new(),
    };
    assert!(retention.append(&identity, &entry).await.is_ok());
    assert!(
        retention
            .recall(&identity, &SpillId::new("x"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(retention.list_entries(&identity).await.unwrap().is_empty());
    assert!(
        rig.store
            .save(&Session::new(identity.clone()))
            .await
            .is_err()
    );
    assert!(rig.store.load(&identity).await.unwrap().is_none());
    assert!(
        rig.store
            .list(&SessionListQuery::All)
            .await
            .unwrap()
            .is_empty()
    );
    let mut accounting = rig.accounting();
    accounting.history_replaced(3);
    assert_eq!(accounting.resets, [3]);
    assert_eq!(
        rig.journal(),
        ["retention.clear", "store.save", "accounting.reset(3)"]
    );
    assert_eq!(rig.cleared_namespaces(), ["cli:rewrite"]);
    assert!(rig.saved().is_empty());
}
