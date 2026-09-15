//! The port fakes and the rig the Save current session tests drive
//! (#1860, D5 #1972): a recording store that can fail or block, fixed
//! workflow and roster sources, and the composed use case over an
//! application-owned active session.
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use super::SaveSession;
use crate::application::durable_prefix::DurablePrefixLatch;
use crate::application::sessions::active_session::{ActiveSessionHandle, ActiveSessionState};
use crate::application::sessions::dto::SessionListQuery;
use crate::application::sessions::ports::{
    HistoricalRosterSource, SessionStore, WorkflowRunSource,
};
use crate::domain::error::DomainError;
use crate::domain::message::Message;
use crate::domain::session::{
    PersistedSubagentRosterEntry, Session, SessionSummary, SubagentLiveness, SubagentRestoreReason,
};
use crate::domain::session_identity::SessionIdentity;
use crate::domain::workflow::WorkflowRunPersisted;

/// What the store was asked to write, in order.
#[derive(Debug, Clone)]
pub(super) enum Write {
    Full(Session),
    Delta {
        previously_persisted: usize,
        messages: Vec<Message>,
        workflow_run: Option<WorkflowRunPersisted>,
    },
    CleanDelta {
        previously_persisted: usize,
        messages: Vec<Message>,
        workflow_run: Option<WorkflowRunPersisted>,
    },
}

#[derive(Default)]
pub(super) struct RecordingStore {
    pub(super) writes: Mutex<Vec<Write>>,
    pub(super) fail_with: Mutex<Option<String>>,
    /// When set, every write waits for a permit before it records.
    pub(super) gate: Option<tokio::sync::Semaphore>,
    /// How many writes were started, gated or not.
    started: std::sync::atomic::AtomicUsize,
}

impl RecordingStore {
    pub(super) fn gated() -> Self {
        Self {
            gate: Some(tokio::sync::Semaphore::new(0)),
            ..Default::default()
        }
    }

    pub(super) fn writes(&self) -> Vec<Write> {
        self.writes.lock().unwrap().clone()
    }

    pub(super) fn started(&self) -> usize {
        self.started.load(std::sync::atomic::Ordering::SeqCst)
    }

    async fn record(&self, write: Write) -> Result<(), DomainError> {
        self.started
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(gate) = &self.gate {
            gate.acquire().await.unwrap().forget();
        }
        if let Some(text) = self.fail_with.lock().unwrap().clone() {
            return Err(DomainError::Session(text));
        }
        self.writes.lock().unwrap().push(write);
        Ok(())
    }
}

pub(super) type Fut<'a, T> = Pin<Box<dyn Future<Output = Result<T, DomainError>> + Send + 'a>>;

impl SessionStore for RecordingStore {
    fn load(&self, _identity: &SessionIdentity) -> Fut<'_, Option<Session>> {
        Box::pin(async { Ok(None) })
    }

    fn save(&self, session: &Session) -> Fut<'_, ()> {
        let session = session.clone();
        Box::pin(async move { self.record(Write::Full(session)).await })
    }

    fn save_delta<'a>(
        &'a self,
        _identity: &'a SessionIdentity,
        messages: &'a [Message],
        previously_persisted: usize,
        workflow_run: Option<WorkflowRunPersisted>,
    ) -> Fut<'a, ()> {
        let messages = messages.to_vec();
        Box::pin(async move {
            self.record(Write::Delta {
                previously_persisted,
                messages,
                workflow_run,
            })
            .await
        })
    }

    fn save_clean_delta<'a>(
        &'a self,
        _identity: &'a SessionIdentity,
        messages: &'a [Message],
        previously_persisted: usize,
        workflow_run: Option<WorkflowRunPersisted>,
    ) -> Fut<'a, ()> {
        let messages = messages.to_vec();
        Box::pin(async move {
            self.record(Write::CleanDelta {
                previously_persisted,
                messages,
                workflow_run,
            })
            .await
        })
    }

    fn exists(&self, _identity: &SessionIdentity) -> Fut<'_, bool> {
        Box::pin(async { Ok(false) })
    }

    fn list(&self, _query: &SessionListQuery) -> Fut<'_, Vec<SessionSummary>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

pub(super) struct FixedWorkflow(Option<WorkflowRunPersisted>);

impl WorkflowRunSource for FixedWorkflow {
    fn persisted_run(&self) -> Option<WorkflowRunPersisted> {
        self.0.clone()
    }
}

pub(super) struct FixedRoster(Vec<PersistedSubagentRosterEntry>);

impl HistoricalRosterSource for FixedRoster {
    fn roster_rows(&self) -> Vec<PersistedSubagentRosterEntry> {
        self.0.clone()
    }
}

pub(super) fn row(id: &str) -> PersistedSubagentRosterEntry {
    PersistedSubagentRosterEntry {
        agent_uuid: id.to_string(),
        display_name: format!("worker-{id}"),
        session_key: id.to_string(),
        liveness: SubagentLiveness::Live,
        restore_reason: SubagentRestoreReason::LegacyUnspecified,
        parent_id: None,
        read_only: false,
        delivered_message_ordinal: None,
        pending_message_reports: Default::default(),
        status: Some("idle".to_string()),
    }
}

pub(super) fn run() -> WorkflowRunPersisted {
    WorkflowRunPersisted {
        template_id: Some("feature".into()),
        done: vec![true, false],
        active_issue: Some((7, "bug".into())),
    }
}

pub(super) struct Rig {
    pub(super) state: ActiveSessionHandle,
    pub(super) store: Arc<RecordingStore>,
    pub(super) latch: Arc<DurablePrefixLatch>,
    pub(super) use_case: Arc<SaveSession>,
}

pub(super) struct RigOptions {
    pub(super) key: &'static str,
    pub(super) prompt: &'static str,
    pub(super) ephemeral: bool,
    pub(super) workflow: Option<WorkflowRunPersisted>,
    pub(super) roster: Option<Vec<PersistedSubagentRosterEntry>>,
    pub(super) store: RecordingStore,
}

impl Default for RigOptions {
    fn default() -> Self {
        Self {
            key: "cli:save",
            prompt: "",
            ephemeral: false,
            workflow: None,
            roster: None,
            store: RecordingStore::default(),
        }
    }
}

pub(super) fn build_rig(options: RigOptions) -> Rig {
    let mut state = ActiveSessionState::new(SessionIdentity::from_persisted_key(options.key));
    state.set_injected_system_prompt(options.prompt);
    let state: ActiveSessionHandle = Arc::new(tokio::sync::RwLock::new(state));
    let store = Arc::new(options.store);
    let latch = DurablePrefixLatch::shared();
    let use_case = Arc::new(SaveSession::new(
        state.clone(),
        store.clone(),
        latch.clone(),
        options
            .workflow
            .map(|run| Arc::new(FixedWorkflow(Some(run))) as Arc<dyn WorkflowRunSource>),
        options
            .roster
            .map(|rows| Arc::new(FixedRoster(rows)) as Arc<dyn HistoricalRosterSource>),
        options.ephemeral,
    ));
    Rig {
        state,
        store,
        latch,
        use_case,
    }
}

pub(super) fn watermark(rig: &Rig) -> usize {
    rig.state.try_read().unwrap().persisted_watermark()
}

pub(super) fn dirty(rig: &Rig) -> bool {
    rig.state.try_read().unwrap().durable_prefix_dirty()
}

pub(super) fn set_watermark(rig: &Rig, n: usize) {
    rig.state.try_write().unwrap().set_persisted_watermark(n);
}

pub(super) fn contents(messages: &[Message]) -> Vec<String> {
    messages.iter().map(|m| m.content.clone()).collect()
}

pub(super) fn ordinals(messages: &[Message]) -> Vec<Option<u64>> {
    messages.iter().map(|m| m.ordinal).collect()
}

#[tokio::test]
async fn the_recording_store_records_fails_and_blocks_as_told() {
    let store = RecordingStore::default();
    let identity = SessionIdentity::from_persisted_key("cli:fake");
    store.save(&Session::new(identity.clone())).await.unwrap();
    store.save_delta(&identity, &[], 0, None).await.unwrap();
    assert!(matches!(
        &store.writes()[..],
        [Write::Full(_), Write::Delta { .. }]
    ));
    assert!(store.load(&identity).await.unwrap().is_none());
    assert!(store.list(&SessionListQuery::All).await.unwrap().is_empty());
    *store.fail_with.lock().unwrap() = Some("no".into());
    assert!(
        store
            .save_clean_delta(&identity, &[], 0, None)
            .await
            .is_err()
    );
    assert_eq!(store.started(), 3);
    let gated = RecordingStore::gated();
    let pending = gated.save(&Session::new(identity.clone()));
    assert!(futures::FutureExt::now_or_never(pending).is_none());
    gated.gate.as_ref().unwrap().add_permits(1);
    gated.save(&Session::new(identity)).await.unwrap();
    assert_eq!(gated.started(), 2, "both attempts reached the gate");
}

#[test]
fn the_fixed_sources_report_what_they_were_given() {
    assert_eq!(FixedWorkflow(Some(run())).persisted_run(), Some(run()));
    assert_eq!(FixedWorkflow(None).persisted_run(), None);
    assert_eq!(FixedRoster(vec![row("a")]).roster_rows().len(), 1);
    assert_eq!(ordinals(&[Message::user("x")]), [None]);
}
