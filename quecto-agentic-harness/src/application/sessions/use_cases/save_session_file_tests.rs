//! #2218 review 3 regressions of the save transaction: over the real file
//! store (a cleared conversation keeps its roster; a write that landed but
//! reported failure is not appended onto) and over the recording rig (the
//! pre-turn save numbers the live transcript; a cancelled save withdraws
//! its ordinals).
use std::sync::{Arc, Mutex};

use futures::FutureExt;

use super::super::SaveSession;
use super::super::rig_tests::{RecordingStore, RigOptions, Write, build_rig, dirty, ordinals, row};
use crate::application::durable_prefix::DurablePrefixLatch;
use crate::application::sessions::active_session::{ActiveSessionHandle, ActiveSessionState};
use crate::application::sessions::dto::SaveTrigger;
use crate::application::sessions::ports::{HistoricalRosterSource, SessionStore};
use crate::domain::error::DomainError;
use crate::domain::message::Message;
use crate::domain::session::{PersistedSubagentRosterEntry, Session};
use crate::domain::session_identity::SessionIdentity;
use crate::domain::workflow::WorkflowRunPersisted;
use crate::infrastructure::persistence::session_layout::FlatSessionLayout;
use crate::infrastructure::persistence::session_store::FileSessionStore;

struct Rows(Vec<PersistedSubagentRosterEntry>);

impl HistoricalRosterSource for Rows {
    fn roster_rows(&self) -> Vec<PersistedSubagentRosterEntry> {
        self.0.clone()
    }
}

fn id() -> SessionIdentity {
    SessionIdentity::from_persisted_key("cli:probe")
}

fn over(store: Arc<dyn SessionStore>, roster: bool) -> (SaveSession, ActiveSessionHandle) {
    let state: ActiveSessionHandle =
        Arc::new(tokio::sync::RwLock::new(ActiveSessionState::new(id())));
    let rows = roster.then(|| Arc::new(Rows(vec![row("a")])) as Arc<dyn HistoricalRosterSource>);
    let save = SaveSession::new(
        state.clone(),
        store,
        DurablePrefixLatch::shared(),
        None,
        rows,
        false,
    );
    (save, state)
}

#[tokio::test]
async fn a_cleared_conversation_keeps_its_roster_on_the_next_turn() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(FileSessionStore::new(FlatSessionLayout::new(tmp.path())));
    let (save, state) = over(store.clone(), true);
    let mut messages = vec![Message::user("hi"), Message::assistant("ok", vec![])];
    save.save(&mut messages, SaveTrigger::Routine)
        .await
        .unwrap();
    // `/clear`: the conversation empties, the watermark resets, and it saves.
    messages.clear();
    state.write().await.set_persisted_watermark(0);
    save.save(&mut messages, SaveTrigger::Routine)
        .await
        .unwrap();
    messages.push(Message::user("again"));
    save.save(&mut messages, SaveTrigger::Routine)
        .await
        .unwrap();
    let loaded = store.load(&id()).await.unwrap().unwrap();
    assert_eq!(
        loaded.subagent_roster.len(),
        1,
        "the roster survives a clear"
    );
}

/// A file store whose next write lands and then reports failure, as a
/// failed `sync_data` or directory fsync does.
struct LandsThenFails {
    inner: FileSessionStore,
    fail_next: Mutex<bool>,
}

impl LandsThenFails {
    fn outcome(&self, landed: Result<(), DomainError>) -> Result<(), DomainError> {
        landed?;
        match std::mem::take(&mut *self.fail_next.lock().unwrap()) {
            true => Err(DomainError::Session("failed to sync session: EIO".into())),
            false => Ok(()),
        }
    }
}

type Fut<'a, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, DomainError>> + Send + 'a>>;

impl SessionStore for LandsThenFails {
    fn claim(&self, identity: &SessionIdentity) -> Result<(), DomainError> {
        self.inner.claim(identity)
    }
    fn release(&self, identity: &SessionIdentity) {
        self.inner.release(identity)
    }
    fn load(&self, identity: &SessionIdentity) -> Fut<'_, Option<Session>> {
        self.inner.load(identity)
    }
    fn save<'a>(&'a self, session: &'a Session) -> Fut<'a, ()> {
        Box::pin(async move { self.outcome(self.inner.save(session).await) })
    }
    fn save_delta<'a>(
        &'a self,
        identity: &'a SessionIdentity,
        messages: &'a [Message],
        previously_persisted: usize,
        workflow_run: Option<WorkflowRunPersisted>,
    ) -> Fut<'a, ()> {
        Box::pin(async move {
            let landed = self
                .inner
                .save_delta(identity, messages, previously_persisted, workflow_run)
                .await;
            self.outcome(landed)
        })
    }
    fn save_clean_delta<'a>(
        &'a self,
        identity: &'a SessionIdentity,
        messages: &'a [Message],
        previously_persisted: usize,
        workflow_run: Option<WorkflowRunPersisted>,
    ) -> Fut<'a, ()> {
        Box::pin(async move {
            let landed = SessionStore::save_clean_delta(
                &self.inner,
                identity,
                messages,
                previously_persisted,
                workflow_run,
            )
            .await;
            self.outcome(landed)
        })
    }
    fn exists(&self, identity: &SessionIdentity) -> Fut<'_, bool> {
        self.inner.exists(identity)
    }
    fn list(
        &self,
        query: &crate::application::sessions::dto::SessionListQuery,
    ) -> Fut<'_, Vec<crate::domain::session::SessionSummary>> {
        self.inner.list(query)
    }
}

#[tokio::test]
async fn a_write_that_landed_but_failed_is_never_appended_onto() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(LandsThenFails {
        inner: FileSessionStore::new(FlatSessionLayout::new(tmp.path())),
        fail_next: Mutex::new(false),
    });
    let (save, _state) = over(store.clone(), false);
    let mut messages = vec![Message::user("one"), Message::assistant("two", vec![])];
    save.save(&mut messages, SaveTrigger::Routine)
        .await
        .unwrap();
    messages.extend([Message::user("three"), Message::assistant("four", vec![])]);
    *store.fail_next.lock().unwrap() = true;
    save.save(&mut messages, SaveTrigger::Routine)
        .await
        .expect_err("the write landed, then failed");
    assert_eq!(ordinals(&messages), [Some(1), Some(2), None, None]);
    for turn in ["five", "six"] {
        messages.push(Message::user(turn));
        save.save(&mut messages, SaveTrigger::Routine)
            .await
            .unwrap();
    }
    let loaded = store.load(&id()).await.unwrap().unwrap();
    let contents: Vec<_> = loaded.messages.iter().map(|m| m.content.as_str()).collect();
    assert_eq!(contents, ["one", "two", "three", "four", "five", "six"]);
    assert_eq!(
        ordinals(&loaded.messages),
        ordinals(&messages),
        "memory and disk agree"
    );
}

#[tokio::test]
async fn a_failed_pre_turn_save_latches_and_the_retry_numbers_the_live_transcript() {
    let rig = build_rig(RigOptions::default());
    let mut messages = vec![Message::user("a"), Message::assistant("b", vec![])];
    *rig.store.fail_with.lock().unwrap() = Some("busy".into());
    let mut pending = Message::user("c");
    rig.use_case
        .save_with_pending_prompt(&mut messages, &mut pending)
        .await
        .expect_err("the store failed");
    assert!(dirty(&rig), "any store error latches the prefix dirty");
    assert_eq!(
        ordinals(&messages),
        [None, None],
        "nothing unheld is served"
    );
    *rig.store.fail_with.lock().unwrap() = None;
    rig.use_case
        .save_with_pending_prompt(&mut messages, &mut pending)
        .await
        .unwrap();
    let [
        Write::Delta {
            messages: written, ..
        },
    ] = &rig.store.writes()[..]
    else {
        panic!("{:?}", rig.store.writes());
    };
    assert_eq!(
        ordinals(&messages),
        [Some(1), Some(2)],
        "the live transcript is numbered"
    );
    assert_eq!(ordinals(written), [Some(1), Some(2), Some(3)]);
    assert_eq!(pending.ordinal, Some(3));
}

#[tokio::test]
async fn a_save_cancelled_mid_write_withdraws_its_ordinals() {
    let rig = build_rig(RigOptions {
        store: RecordingStore::gated(),
        ..Default::default()
    });
    let mut durable = Message::user("durable");
    durable.ordinal = Some(4);
    let mut messages = vec![durable, Message::assistant("new", vec![])];
    {
        let save = rig.use_case.save(&mut messages, SaveTrigger::OrdinaryExit);
        assert!(
            save.now_or_never().is_none(),
            "the gated store holds the write"
        );
    }
    assert_eq!(messages.len(), 2, "the lent transcript came back");
    assert_eq!(
        ordinals(&messages),
        [Some(4), None],
        "only this save's ordinal is withdrawn"
    );
}

/// A write that lands and then never returns: its save is dropped.
struct LandsThenHangs {
    inner: FileSessionStore,
    hang_next: Mutex<bool>,
}

impl LandsThenHangs {
    async fn outcome(&self, landed: Result<(), DomainError>) -> Result<(), DomainError> {
        landed?;
        if std::mem::take(&mut *self.hang_next.lock().unwrap()) {
            std::future::pending::<()>().await;
        }
        Ok(())
    }
}

impl SessionStore for LandsThenHangs {
    fn claim(&self, identity: &SessionIdentity) -> Result<(), DomainError> {
        self.inner.claim(identity)
    }
    fn release(&self, identity: &SessionIdentity) {
        self.inner.release(identity)
    }
    fn load(&self, identity: &SessionIdentity) -> Fut<'_, Option<Session>> {
        self.inner.load(identity)
    }
    fn save<'a>(&'a self, session: &'a Session) -> Fut<'a, ()> {
        Box::pin(async move { self.outcome(self.inner.save(session).await).await })
    }
    fn save_delta<'a>(
        &'a self,
        identity: &'a SessionIdentity,
        messages: &'a [Message],
        previously_persisted: usize,
        workflow_run: Option<WorkflowRunPersisted>,
    ) -> Fut<'a, ()> {
        Box::pin(async move {
            let landed = self
                .inner
                .save_delta(identity, messages, previously_persisted, workflow_run)
                .await;
            self.outcome(landed).await
        })
    }
    fn save_clean_delta<'a>(
        &'a self,
        identity: &'a SessionIdentity,
        messages: &'a [Message],
        previously_persisted: usize,
        workflow_run: Option<WorkflowRunPersisted>,
    ) -> Fut<'a, ()> {
        Box::pin(async move {
            let landed = SessionStore::save_clean_delta(
                &self.inner,
                identity,
                messages,
                previously_persisted,
                workflow_run,
            )
            .await;
            self.outcome(landed).await
        })
    }
    fn exists(&self, identity: &SessionIdentity) -> Fut<'_, bool> {
        self.inner.exists(identity)
    }
    fn list(
        &self,
        query: &crate::application::sessions::dto::SessionListQuery,
    ) -> Fut<'_, Vec<crate::domain::session::SessionSummary>> {
        self.inner.list(query)
    }
}

/// #2218 review 4 (probe D): a save cancelled after its write landed marks
/// the loop unsettled, so the next save verifies the file and no later
/// turn is appended at the stale watermark and lost.
#[tokio::test]
async fn a_save_cancelled_after_its_write_landed_loses_no_later_turn() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(LandsThenHangs {
        inner: FileSessionStore::new(FlatSessionLayout::new(tmp.path())),
        hang_next: Mutex::new(false),
    });
    let (save, _state) = over(store.clone(), false);
    let mut messages = vec![Message::user("one"), Message::assistant("two", vec![])];
    save.save(&mut messages, SaveTrigger::Routine)
        .await
        .unwrap();
    messages.extend([Message::user("three"), Message::assistant("four", vec![])]);
    *store.hang_next.lock().unwrap() = true;
    let cancelled = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        save.save(&mut messages, SaveTrigger::Routine),
    )
    .await;
    assert!(cancelled.is_err(), "the save was dropped mid-write");
    assert_eq!(ordinals(&messages), [Some(1), Some(2), None, None]);
    for turn in ["five", "six"] {
        messages.push(Message::user(turn));
        save.save(&mut messages, SaveTrigger::Routine)
            .await
            .unwrap();
    }
    store.inner.release(&id());
    let fresh = FileSessionStore::new(FlatSessionLayout::new(tmp.path()));
    let loaded = fresh.load(&id()).await.unwrap().unwrap();
    let contents: Vec<_> = loaded.messages.iter().map(|m| m.content.as_str()).collect();
    assert_eq!(contents, ["one", "two", "three", "four", "five", "six"]);
}

/// #2218 review 4 (probe F, "fsyncgate"): after an fsync failure the page
/// cache may hold what never reached the disk, so the next save compacts
/// the whole transcript instead of appending onto what it reads back.
#[tokio::test]
async fn the_save_after_a_failed_fsync_compacts() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(FileSessionStore::new(FlatSessionLayout::new(tmp.path())));
    let (save, _state) = over(store.clone(), false);
    let mut messages = vec![Message::user("one"), Message::assistant("two", vec![])];
    save.save(&mut messages, SaveTrigger::Routine)
        .await
        .unwrap();
    messages.push(Message::user("three"));
    store.fail_next_sync(&id());
    save.save(&mut messages, SaveTrigger::Routine)
        .await
        .expect_err("the fsync failed");
    let path = tmp.path().join("sessions").join("cli_probe.json");
    assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 2);
    messages.push(Message::user("four"));
    save.save(&mut messages, SaveTrigger::Routine)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap().lines().count(),
        1,
        "the whole transcript was rewritten, not appended"
    );
    let loaded = store.load(&id()).await.unwrap().unwrap();
    assert_eq!(ordinals(&loaded.messages), ordinals(&messages));
}
