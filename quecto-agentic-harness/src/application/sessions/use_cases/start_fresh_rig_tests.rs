//! The rig the Start fresh conversation tests drive (D7 #1976): a store,
//! a retention store, a fleet, a roster, an identity generator and a
//! switch-runtime fake that all write into one shared journal, so the
//! exact order in which the transaction settled, saved, replaced,
//! released, generated, propagated, switched and cleared is observable —
//! and so a `claim` of the fresh identity, which must never happen, would
//! be journaled too. The store's `release` and the runtime's key
//! propagation also journal the active session's identity at call time,
//! so a switch that ran too early is visible in the journal.
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::{DepartingChildren, ResumeSavedSession, SaveSession, StartFreshConversation};
use crate::application::durable_prefix::DurablePrefixLatch;
use crate::application::sessions::active_session::{ActiveSessionHandle, ActiveSessionState};
use crate::application::sessions::dto::{FleetSettled, FleetSettlementOutcome, SessionListQuery};
use crate::application::sessions::ports::session_runtime::TurnAccountingReset;
use crate::application::sessions::ports::{
    ContextSpillStore, DelegatedChildrenRoster, FleetSettlement, FreshSessionIdentityGenerator,
    SessionKeyPropagation, SessionStore, SessionSwitchRuntime, SpillIndexList,
};
use crate::domain::error::DomainError;
use crate::domain::message::Message;
use crate::domain::session::{Session, SessionSummary, SpillEntries, SpillEntry};
use crate::domain::session_identity::{SessionIdentity, SpillId};
use crate::domain::workflow::WorkflowRunPersisted;

pub(crate) type Journal = Arc<Mutex<Vec<String>>>;
type Fut<'a, T> = Pin<Box<dyn Future<Output = Result<T, DomainError>> + Send + 'a>>;

pub(crate) const OLD_KEY: &str = "cli:departing";
pub(crate) const FRESH_KEY: &str = "chat-1700000000-2a";

fn note(journal: &Journal, entry: impl Into<String>) {
    journal.lock().unwrap().push(entry.into());
}

fn active_identity(state: &Option<ActiveSessionHandle>) -> String {
    state
        .as_ref()
        .map(|state| {
            state
                .try_read()
                .unwrap()
                .identity()
                .runtime_key()
                .to_string()
        })
        .unwrap_or_default()
}

/// A store that journals every write, claim, load and release (a release
/// with the identity the active session stands for at that moment). It
/// holds the sessions it was seeded with (the resume targets, D8 #1977) and
/// can be told to refuse a claim of one key or to fail every load.
pub(crate) struct RecordingStore {
    journal: Journal,
    fail_save: bool,
    state: Mutex<Option<ActiveSessionHandle>>,
    pub(crate) saved: Mutex<Vec<Vec<Message>>>,
    pub(crate) claimed: Mutex<Vec<String>>,
    pub(crate) released: Mutex<Vec<String>>,
    pub(crate) sessions: Mutex<Vec<Session>>,
    pub(crate) owned_elsewhere: Mutex<Option<String>>,
    pub(crate) fail_load: AtomicBool,
}

impl RecordingStore {
    pub(crate) fn seed(&self, session: Session) {
        self.sessions.lock().unwrap().push(session);
    }

    fn write(&self, op: &str, messages: &[Message]) -> Fut<'_, ()> {
        note(&self.journal, format!("store.{op}"));
        let messages = messages.to_vec();
        let fail = self.fail_save;
        Box::pin(async move {
            if fail {
                return Err(DomainError::Session("disk full".into()));
            }
            self.saved.lock().unwrap().push(messages);
            Ok(())
        })
    }
}

impl SessionStore for RecordingStore {
    fn claim(&self, identity: &SessionIdentity) -> Result<(), DomainError> {
        note(
            &self.journal,
            format!("store.claim({})", identity.runtime_key()),
        );
        self.claimed
            .lock()
            .unwrap()
            .push(identity.runtime_key().to_string());
        if self.owned_elsewhere.lock().unwrap().as_deref() == Some(identity.runtime_key()) {
            return Err(DomainError::Session(format!(
                "session {} is owned by another live process",
                identity.runtime_key()
            )));
        }
        Ok(())
    }
    fn release(&self, identity: &SessionIdentity) {
        let active = active_identity(&self.state.lock().unwrap());
        note(
            &self.journal,
            format!("store.release({})@active={active}", identity.runtime_key()),
        );
        self.released
            .lock()
            .unwrap()
            .push(identity.runtime_key().to_string());
    }
    fn load(&self, identity: &SessionIdentity) -> Fut<'_, Option<Session>> {
        note(
            &self.journal,
            format!("store.load({})", identity.runtime_key()),
        );
        if self.fail_load.load(Ordering::SeqCst) {
            return Box::pin(async { Err(DomainError::Session("corrupt session file".into())) });
        }
        let found = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .find(|session| &session.key == identity)
            .cloned();
        Box::pin(async { Ok(found) })
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

/// A retention store that journals every clear with its namespace.
pub(crate) struct RecordingRetention {
    journal: Journal,
    fail: bool,
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
        note(
            &self.journal,
            format!("retention.clear({})", identity.runtime_key()),
        );
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

/// Hands out `FRESH_KEY` (or whatever it was told) and journals it.
pub(crate) struct FixedIdentities {
    journal: Journal,
    identity: SessionIdentity,
}

impl FreshSessionIdentityGenerator for FixedIdentities {
    fn fresh_identity(&self) -> SessionIdentity {
        note(
            &self.journal,
            format!("identity.generate({})", self.identity.runtime_key()),
        );
        self.identity.clone()
    }
}

/// A fleet whose outcome is scripted; journals every run.
pub(crate) struct ScriptedFleet {
    journal: Journal,
    outcome: FleetSettlementOutcome,
}

impl FleetSettlement for ScriptedFleet {
    fn settle_fleet(&self) -> Pin<Box<dyn Future<Output = FleetSettlementOutcome> + Send + '_>> {
        note(&self.journal, "fleet.settle");
        let outcome = self.outcome.clone();
        Box::pin(async move { outcome })
    }
}

/// A roster with a settable live-row count; journals every clear.
pub(crate) struct FakeRoster {
    journal: Journal,
    pub(crate) live: AtomicUsize,
    pub(crate) records: AtomicUsize,
}

impl DelegatedChildrenRoster for FakeRoster {
    fn live_delegated_rows(&self) -> usize {
        self.live.load(Ordering::SeqCst)
    }
    fn clear_roster(&self) -> usize {
        note(&self.journal, "roster.clear");
        self.live.store(0, Ordering::SeqCst);
        self.records.swap(0, Ordering::SeqCst)
    }
}

/// Journals every runtime effect with its argument (the key propagation
/// with the identity the active session stands for at that moment).
pub(crate) struct RecordingRuntime {
    journal: Journal,
    state: Option<ActiveSessionHandle>,
    pub(crate) propagated: Vec<String>,
    pub(crate) resets: Vec<usize>,
}

impl TurnAccountingReset for RecordingRuntime {
    fn history_replaced(&mut self, visible_message_count: usize) {
        note(
            &self.journal,
            format!("accounting.reset({visible_message_count})"),
        );
        self.resets.push(visible_message_count);
    }
}

impl SessionKeyPropagation for RecordingRuntime {
    fn session_key_changed(&mut self, identity: &SessionIdentity) {
        let active = active_identity(&self.state);
        note(
            &self.journal,
            format!("key.propagate({})@active={active}", identity.runtime_key()),
        );
        self.propagated.push(identity.runtime_key().to_string());
    }
}

impl SessionSwitchRuntime for RecordingRuntime {
    fn reset_effort_to_default(&mut self) {
        note(&self.journal, "effort.reset");
    }
    fn reset_workflow(&mut self) {
        note(&self.journal, "workflow.reset");
    }
    fn restore_workflow(&mut self, run: WorkflowRunPersisted) {
        note(
            &self.journal,
            format!("workflow.restore({})", run.template_id.unwrap_or_default()),
        );
    }
}

pub(crate) struct FreshRig {
    pub(crate) state: ActiveSessionHandle,
    pub(crate) store: Arc<RecordingStore>,
    pub(crate) roster: Option<Arc<FakeRoster>>,
    pub(crate) journal: Journal,
    pub(crate) fresh: StartFreshConversation,
    /// The resume transaction over the same graph (D8 #1977).
    pub(crate) resume: ResumeSavedSession,
    pub(crate) children: Arc<DepartingChildren>,
}

pub(crate) struct FreshOptions {
    pub(crate) prompt: &'static str,
    /// `Some(fail)` composes a retention store that fails when told;
    /// `None` composes none at all.
    pub(crate) retention: Option<bool>,
    pub(crate) save_fails: bool,
    pub(crate) ephemeral: bool,
    /// `Some((live, records))` composes a roster; `None` tracks none.
    pub(crate) roster: Option<(usize, usize)>,
    pub(crate) fresh_identity: &'static str,
    pub(crate) current_identity: &'static str,
}

impl Default for FreshOptions {
    fn default() -> Self {
        Self {
            prompt: "",
            retention: Some(false),
            save_fails: false,
            ephemeral: false,
            roster: Some((0, 0)),
            fresh_identity: FRESH_KEY,
            current_identity: OLD_KEY,
        }
    }
}

impl FreshRig {
    pub(crate) fn runtime(&self) -> RecordingRuntime {
        RecordingRuntime {
            journal: self.journal.clone(),
            state: Some(self.state.clone()),
            propagated: Vec::new(),
            resets: Vec::new(),
        }
    }

    pub(crate) fn fleet(&self, outcome: FleetSettlementOutcome) -> ScriptedFleet {
        ScriptedFleet {
            journal: self.journal.clone(),
            outcome,
        }
    }

    pub(crate) fn settled_fleet(&self) -> ScriptedFleet {
        self.fleet(FleetSettlementOutcome::Settled(FleetSettled {
            settled: 1,
            pruned: 1,
            joined: false,
            removed: 1,
        }))
    }

    pub(crate) fn journal(&self) -> Vec<String> {
        self.journal.lock().unwrap().clone()
    }

    pub(crate) fn identity(&self) -> String {
        self.state
            .try_read()
            .unwrap()
            .identity()
            .runtime_key()
            .to_string()
    }

    pub(crate) fn watermark(&self) -> usize {
        self.state.try_read().unwrap().persisted_watermark()
    }

    pub(crate) fn set_watermark(&self, n: usize) {
        self.state.try_write().unwrap().set_persisted_watermark(n);
    }

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

    pub(crate) fn epoch(&self) -> u64 {
        self.state.try_read().unwrap().conversation().epoch()
    }

    pub(crate) fn live_contents(&self) -> Vec<String> {
        self.state
            .try_read()
            .unwrap()
            .conversation()
            .live_messages()
            .iter()
            .map(|m| m.content.clone())
            .collect()
    }
}

pub(crate) fn build_fresh_rig(options: FreshOptions) -> FreshRig {
    let journal: Journal = Arc::default();
    let mut state = ActiveSessionState::new(SessionIdentity::from_persisted_key(
        options.current_identity,
    ));
    state.set_injected_system_prompt(options.prompt);
    let retention = options.retention.map(|fail| {
        Arc::new(RecordingRetention {
            journal: journal.clone(),
            fail,
        }) as Arc<dyn ContextSpillStore>
    });
    state.set_spill_store(retention);
    let state: ActiveSessionHandle = Arc::new(tokio::sync::RwLock::new(state));
    let store = Arc::new(RecordingStore {
        journal: journal.clone(),
        fail_save: options.save_fails,
        state: Mutex::new(Some(state.clone())),
        saved: Mutex::new(Vec::new()),
        claimed: Mutex::new(Vec::new()),
        released: Mutex::new(Vec::new()),
        sessions: Mutex::new(Vec::new()),
        owned_elsewhere: Mutex::new(None),
        fail_load: AtomicBool::new(false),
    });
    let save = Arc::new(SaveSession::new(
        state.clone(),
        store.clone(),
        DurablePrefixLatch::shared(),
        None,
        None,
        options.ephemeral,
    ));
    let roster = options.roster.map(|(live, records)| {
        Arc::new(FakeRoster {
            journal: journal.clone(),
            live: AtomicUsize::new(live),
            records: AtomicUsize::new(records),
        })
    });
    let children = Arc::new(DepartingChildren::new(
        roster
            .clone()
            .map(|r| r as Arc<dyn DelegatedChildrenRoster>),
    ));
    let identities = Arc::new(FixedIdentities {
        journal: journal.clone(),
        identity: SessionIdentity::from_persisted_key(options.fresh_identity),
    });
    FreshRig {
        fresh: StartFreshConversation::new(
            state.clone(),
            save.clone(),
            store.clone(),
            identities,
            children.clone(),
        ),
        resume: ResumeSavedSession::new(
            state.clone(),
            save,
            store.clone(),
            children.clone(),
            options.ephemeral,
        ),
        state,
        store,
        roster,
        journal,
        children,
    }
}

#[tokio::test]
async fn the_fakes_journal_and_fail_as_told() {
    let rig = build_fresh_rig(FreshOptions {
        retention: Some(true),
        save_fails: true,
        roster: Some((2, 3)),
        ..FreshOptions::default()
    });
    let identity = SessionIdentity::from_persisted_key(OLD_KEY);
    assert!(rig.store.claim(&identity).is_ok());
    rig.store.release(&identity);
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
    let retention = rig
        .state
        .read()
        .await
        .conversation()
        .spill_store()
        .cloned()
        .unwrap();
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
    let roster = rig.roster.clone().unwrap();
    assert_eq!(roster.live_delegated_rows(), 2);
    assert_eq!(roster.clear_roster(), 3);
    assert_eq!(roster.live_delegated_rows(), 0);
    let mut runtime = rig.runtime();
    runtime.history_replaced(4);
    runtime.session_key_changed(&identity);
    runtime.reset_effort_to_default();
    runtime.reset_workflow();
    runtime.restore_workflow(WorkflowRunPersisted {
        template_id: Some("feature".into()),
        done: vec![],
        active_issue: None,
    });
    assert_eq!(runtime.resets, [4]);
    assert_eq!(runtime.propagated, [OLD_KEY]);
    assert!(matches!(
        rig.fleet(FleetSettlementOutcome::Interrupted)
            .settle_fleet()
            .await,
        FleetSettlementOutcome::Interrupted
    ));
    assert_eq!(
        rig.journal(),
        [
            "store.claim(cli:departing)",
            "store.release(cli:departing)@active=cli:departing",
            "store.save",
            "store.load(cli:departing)",
            "retention.clear(cli:departing)",
            "roster.clear",
            "accounting.reset(4)",
            "key.propagate(cli:departing)@active=cli:departing",
            "effort.reset",
            "workflow.reset",
            "workflow.restore(feature)",
            "fleet.settle",
        ]
    );
    assert_eq!(rig.store.claimed.lock().unwrap().as_slice(), [OLD_KEY]);
    assert_eq!(rig.store.released.lock().unwrap().as_slice(), [OLD_KEY]);
    assert!(rig.store.saved.lock().unwrap().is_empty());
    // Seeded sessions load; a claim of the key owned elsewhere is refused;
    // a failing load fails every key.
    rig.store.seed(Session::new(identity.clone()));
    assert!(rig.store.load(&identity).await.unwrap().is_some());
    *rig.store.owned_elsewhere.lock().unwrap() = Some(OLD_KEY.into());
    assert!(rig.store.claim(&identity).is_err());
    rig.store.fail_load.store(true, Ordering::SeqCst);
    assert!(rig.store.load(&identity).await.is_err());
    assert!(format!("{:?}", rig.children).contains("tracks_roster: true"));
    assert!(format!("{:?}", rig.fresh).starts_with("StartFreshConversation"));
}
