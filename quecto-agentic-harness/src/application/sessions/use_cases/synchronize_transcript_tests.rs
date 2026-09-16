use super::SynchronizeTranscript;
use crate::application::sessions::active_session::{ActiveSessionHandle, ActiveSessionState};
use crate::application::sessions::conversation_ledger::{LEDGER_MAX_ENTRIES, LedgerAdvance};
use crate::application::sessions::dto::{SyncRequest, TranscriptDelta, TranscriptSync};
use crate::domain::message::Message;
use crate::domain::session_identity::SessionIdentity;
use std::sync::Arc;

const RESET_WINDOW: usize = 64;

struct Rig {
    state: ActiveSessionHandle,
    synchronize: SynchronizeTranscript,
}

fn rig() -> Rig {
    let state: ActiveSessionHandle = Arc::new(tokio::sync::RwLock::new(ActiveSessionState::new(
        SessionIdentity::ephemeral(),
    )));
    Rig {
        synchronize: SynchronizeTranscript::new(state.clone()),
        state,
    }
}

impl Rig {
    fn publish(&self, messages: &[Message]) -> LedgerAdvance {
        self.state
            .try_write()
            .expect("uncontended")
            .publish(messages)
    }

    fn clear(&self) -> LedgerAdvance {
        self.state.try_write().expect("uncontended").clear()
    }

    fn position(&self) -> (u64, u64) {
        let state = self.state.try_read().expect("uncontended");
        (state.conversation().epoch(), state.conversation().rev())
    }

    async fn sync(&self, epoch: u64, since_rev: u64) -> TranscriptSync {
        self.sync_carrying(epoch, since_rev, |_| true).await
    }

    async fn sync_carrying(
        &self,
        epoch: u64,
        since_rev: u64,
        carry: impl FnMut(&Message) -> bool,
    ) -> TranscriptSync {
        let request = SyncRequest {
            epoch,
            since_rev,
            reset_window: RESET_WINDOW,
        };
        self.synchronize.execute(&request, carry).await
    }
}

fn messages(n: usize) -> Vec<Message> {
    (0..n)
        .map(|i| Message::user(format!("message-{i}")))
        .collect()
}

fn delta(sync: TranscriptSync) -> TranscriptDelta {
    match sync {
        TranscriptSync::Delta(delta) => delta,
        TranscriptSync::Reset(reset) => panic!("expected a delta, got a reset at {reset:?}"),
    }
}

fn ids(messages: &[Message]) -> Vec<String> {
    messages.iter().map(|m| m.id().to_string()).collect()
}

#[tokio::test]
async fn a_delta_is_chronological_and_exclusive_of_the_client_revision() {
    let rig = rig();
    let published = messages(3);
    let advance = rig.publish(&published);
    assert_eq!(advance.rev, 3);
    let delta = delta(rig.sync(0, 1).await);
    assert_eq!((delta.epoch, delta.rev), (0, 3));
    assert_eq!(ids(&delta.messages), ids(&published[1..]));
    assert!(delta.caught_up());
    assert_eq!(delta.next_rev, None);
}

#[tokio::test]
async fn a_client_at_the_newest_revision_gets_an_empty_caught_up_delta() {
    let rig = rig();
    rig.publish(&messages(3));
    let sync = delta(rig.sync(0, 3).await);
    assert!(sync.caught_up());
    assert_eq!((sync.epoch, sync.rev), (0, 3));
    assert!(sync.messages.is_empty());
    let whole = delta(rig.sync(0, 1).await);
    assert_eq!(whole.messages.len(), 2, "the oldest retained revision is 1");
}

#[tokio::test]
async fn a_client_before_the_first_committed_revision_is_told_to_resynchronise() {
    let rig = rig();
    let empty = delta(rig.sync(0, 0).await);
    assert!(empty.messages.is_empty(), "nothing committed yet");
    rig.publish(&messages(3));
    let TranscriptSync::Reset(reset) = rig.sync(0, 0).await else {
        panic!("revision 0 lies below the oldest retained revision, 1");
    };
    assert_eq!(reset.page.messages.len(), 3);
}

#[tokio::test]
async fn a_changed_epoch_demands_a_reset_at_the_current_position() {
    let rig = rig();
    rig.publish(&[Message::user("old")]);
    let (epoch, rev) = rig.position();
    let cleared = rig.clear();
    assert_eq!((cleared.epoch, cleared.rev), (epoch + 1, rev));
    let stale = rig.sync(epoch, rev).await;
    let TranscriptSync::Reset(reset) = stale else {
        panic!("a stale epoch is told to resynchronise");
    };
    assert_eq!((reset.epoch, reset.rev), (cleared.epoch, rev));
    assert!(reset.page.messages.is_empty());
    assert!(!reset.page.has_more_before);
    let current = delta(rig.sync(cleared.epoch, cleared.rev).await);
    assert!(current.caught_up());
}

#[tokio::test]
async fn a_revision_below_the_retained_frontier_demands_a_reset() {
    let rig = rig();
    let published = messages(LEDGER_MAX_ENTRIES + 1);
    let evicted = published[0].id().to_string();
    let oldest_retained = published[1].id().to_string();
    rig.publish(&published);
    // Revision 1 committed the evicted message; the oldest retained
    // revision is 2, so a client at 1 still reconstructs its delta.
    let retained = delta(rig.sync(0, 1).await);
    assert_eq!(retained.messages[0].id().to_string(), oldest_retained);
    assert_eq!(retained.messages.len(), LEDGER_MAX_ENTRIES);
    let TranscriptSync::Reset(reset) = rig.sync(0, 0).await else {
        panic!("a revision below the frontier is told to resynchronise");
    };
    assert_eq!(reset.page.messages.len(), RESET_WINDOW);
    assert!(reset.page.has_more_before);
    assert!(
        reset
            .page
            .messages
            .iter()
            .all(|m| m.id().to_string() != evicted)
    );
}

#[tokio::test]
async fn a_reset_carries_the_newest_window_as_a_history_page() {
    let rig = rig();
    let published = messages(RESET_WINDOW + 1);
    rig.publish(&published);
    let (epoch, rev) = rig.position();
    let TranscriptSync::Reset(reset) = rig.sync(epoch.wrapping_add(1), rev).await else {
        panic!("a reset");
    };
    assert_eq!(reset.page.messages.len(), RESET_WINDOW);
    assert_eq!(ids(&reset.page.messages), ids(&published[1..]));
    assert!(reset.page.has_more_before);
    assert_eq!(
        reset.page.before.as_ref().map(|id| id.as_str().to_string()),
        Some(published[1].id().to_string())
    );
}

#[tokio::test]
async fn the_frame_predicate_cuts_the_delta_at_the_first_refusal() {
    let rig = rig();
    let published = messages(5);
    rig.publish(&published);
    let mut offered = Vec::new();
    let delta = delta(
        rig.sync_carrying(0, 1, |message| {
            offered.push(message.id().to_string());
            offered.len() <= 2
        })
        .await,
    );
    assert_eq!(ids(&delta.messages), ids(&published[1..3]));
    assert_eq!(
        delta.next_rev,
        Some(4),
        "the first refused message's revision"
    );
    assert!(!delta.caught_up());
    assert_eq!(
        offered,
        ids(&published[1..4]),
        "nothing after the refusal is offered"
    );
}

#[tokio::test]
async fn a_refused_first_message_yields_an_empty_cut_delta() {
    let rig = rig();
    rig.publish(&messages(3));
    let delta = delta(rig.sync_carrying(0, 1, |_| false).await);
    assert!(delta.messages.is_empty());
    assert_eq!(delta.next_rev, Some(2));
    assert_eq!(delta.rev, 3);
}

#[tokio::test]
async fn a_collapsed_republish_keeps_the_message_id_and_a_reset_shows_the_live_stub() {
    let rig = rig();
    let full = Message::assistant("full content", vec![]);
    let id = full.id().to_string();
    let first = rig.publish(std::slice::from_ref(&full));
    let mut stub = full.clone();
    stub.content = "stub".into();
    stub.is_collapsed = true;
    let second = rig.publish(std::slice::from_ref(&stub));
    assert!(!second.changed);
    assert_eq!(second.rev, first.rev);
    let TranscriptSync::Reset(reset) = rig.sync(0, first.rev - 1).await else {
        panic!("a revision below the first commit resets to the live transcript");
    };
    assert_eq!(reset.page.messages[0].id().to_string(), id);
    assert_eq!(reset.page.messages[0].content, "stub");
}

#[tokio::test]
async fn a_delta_serves_the_retained_full_copy_of_a_collapsed_message() {
    let rig = rig();
    let earlier = Message::user("earlier");
    let full = Message::assistant("full content", vec![]);
    rig.publish(std::slice::from_ref(&earlier));
    let committed = rig.publish(&[earlier.clone(), full.clone()]);
    let mut stub = full.clone();
    stub.content = "stub".into();
    stub.is_collapsed = true;
    rig.publish(&[earlier, stub]);
    let delta = delta(rig.sync(0, committed.rev - 1).await);
    assert_eq!(delta.messages.len(), 1);
    assert_eq!(delta.messages[0].id().to_string(), full.id().to_string());
    assert_eq!(
        delta.messages[0].content, "full content",
        "the ledger's full copy outranks the live stub"
    );
}

#[test]
fn the_use_case_debugs_by_name() {
    assert!(format!("{:?}", rig().synchronize).starts_with("SynchronizeTranscript"));
}
