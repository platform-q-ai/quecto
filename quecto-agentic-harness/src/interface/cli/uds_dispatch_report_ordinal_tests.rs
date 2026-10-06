//! A finished child's report is durably numbered (#2218): every turn the
//! loop runs — a `prompt`, a queued `follow_up` (how a spawn hands a child
//! its task), drained pending work, a nudge — ends with the routine save,
//! so the messages `get_messages` then serves carry their durable ordinals
//! and a supervisor's unread cursor can advance. A drain that runs no turn
//! saves nothing.
use super::fixture_tests::Fixture;
use crate::application::sessions::ports::SessionStore;
use crate::domain::message::{Message, Role};
use crate::domain::sessions::entities::session_identity::SessionIdentity;

pub(super) fn assert_every_visible_message_numbered(fx: &Fixture) {
    assert!(!fx.messages.is_empty(), "the turn must have run");
    for message in visible(fx) {
        assert!(
            message.ordinal.is_some(),
            "{:?} message {:?} has no durable ordinal after its turn",
            message.role,
            message.content
        );
    }
}

fn visible(fx: &Fixture) -> Vec<&Message> {
    let injected = fx.system_prompt.as_str();
    fx.messages
        .iter()
        .filter(|m| injected.is_empty() || !(m.role == Role::System && m.content == injected))
        .collect()
}

pub(super) async fn persisted(fx: &Fixture) -> Option<Vec<Message>> {
    fx.store
        .load(&SessionIdentity::from_persisted_key(&fx.session_key))
        .await
        .expect("the store reads")
        .map(|session| session.messages)
}

async fn persisted_ordinals(fx: &Fixture) -> Vec<Option<u64>> {
    persisted(fx)
        .await
        .expect("the turn persisted the session")
        .iter()
        .map(|message| message.ordinal)
        .collect()
}

async fn follow_up(fx: &mut Fixture, task: &str) {
    let mut ctx = fx.ctx();
    super::handle_follow_up(&mut ctx, Some("task"), "follow_up", task.into()).await;
}

#[tokio::test]
async fn a_follow_up_turn_at_idle_leaves_every_message_durably_numbered() {
    let mut fx = Fixture::new();
    follow_up(&mut fx, "reply ok").await;
    assert_every_visible_message_numbered(&fx);
    let live: Vec<Option<u64>> = fx.messages.iter().map(|m| m.ordinal).collect();
    assert_eq!(
        persisted_ordinals(&fx).await,
        live,
        "the store holds the ordinals the live transcript serves"
    );
}

#[tokio::test]
async fn an_injected_system_prompt_is_never_numbered_or_written() {
    let mut fx = Fixture::new().with_system_prompt("be helpful");
    fx.messages = vec![Message::system("be helpful")];
    follow_up(&mut fx, "reply ok").await;
    assert_every_visible_message_numbered(&fx);
    assert_eq!(fx.messages[0].content, "be helpful");
    assert_eq!(
        fx.messages[0].ordinal, None,
        "the injected prompt is not history"
    );
    let stored = persisted(&fx).await.expect("the turn persisted");
    assert!(stored.iter().all(|m| m.content != "be helpful"));
    assert_eq!(stored.len(), fx.messages.len() - 1);
}

#[tokio::test]
async fn drained_pending_work_at_idle_is_durably_numbered() {
    let mut fx = Fixture::new();
    fx.session
        .enqueue_pending("a completed sub-agent's note".into());
    {
        let mut ctx = fx.ctx();
        super::super::drain_pending_and_nudge(&mut ctx).await;
    }
    assert_every_visible_message_numbered(&fx);
    assert_eq!(persisted_ordinals(&fx).await.len(), fx.messages.len());
}

#[tokio::test]
async fn work_drained_behind_a_stale_abort_is_durably_numbered() {
    use crate::interface::cli::uds_cancel::CancelSlot;
    let mut fx = Fixture::new();
    fx.session
        .enqueue_pending("queued before the stale abort".into());
    *fx.cancel.lock().unwrap() = CancelSlot::Fired;
    {
        let mut ctx = fx.ctx();
        super::super::handle_prompt(
            &mut ctx,
            super::super::PromptCommand {
                id: Some("p".into()),
                type_name: "prompt".into(),
                message: "cancelled before it starts".into(),
                streaming_behavior: None,
            },
        )
        .await;
    }
    assert!(
        fx.messages
            .iter()
            .all(|m| m.content != "cancelled before it starts"),
        "the pre-cancelled prompt itself never runs"
    );
    assert_every_visible_message_numbered(&fx);
    assert_eq!(persisted_ordinals(&fx).await.len(), fx.messages.len());
}

#[tokio::test]
async fn a_prompt_is_saved_before_its_snapshots_publish() {
    let mut fx = Fixture::new();
    {
        let mut ctx = fx.ctx();
        super::super::handle_prompt(
            &mut ctx,
            super::super::PromptCommand {
                id: Some("p".into()),
                type_name: "prompt".into(),
                message: "reply ok".into(),
                streaming_behavior: None,
            },
        )
        .await;
    }
    let published = fx
        .sessions
        .read_handles()
        .read_history
        .newest_live_page(64)
        .await;
    assert_eq!(published.messages.len(), fx.messages.len());
    assert!(
        published.messages.iter().all(|m| m.ordinal.is_some()),
        "the published transcript carries the durable ordinals: {:?}",
        published
            .messages
            .iter()
            .map(|m| m.ordinal)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn a_drained_turn_is_saved_before_its_snapshots_publish() {
    let mut fx = Fixture::new();
    // An uncorrelated note: its turn's own publish is the last one.
    fx.session
        .enqueue_pending("a completed sub-agent's note".into());
    {
        let mut ctx = fx.ctx();
        super::super::drain_pending_and_nudge(&mut ctx).await;
    }
    let published = fx
        .sessions
        .read_handles()
        .read_history
        .newest_live_page(64)
        .await;
    assert_eq!(published.messages.len(), fx.messages.len());
    assert!(
        published.messages.iter().all(|m| m.ordinal.is_some()),
        "a busy reader's snapshot carries the durable ordinals"
    );
}

#[tokio::test]
async fn a_drain_that_yields_to_a_steer_saves_nothing() {
    let mut fx = Fixture::new();
    fx.messages = vec![Message::user("not yet saved")];
    fx.session.enqueue_pending("queued work".into());
    {
        let mut ctx = fx.ctx();
        ctx.turn_control.mark_steer();
        super::super::drain_pending_and_nudge(&mut ctx).await;
    }
    assert_eq!(fx.messages.len(), 1, "the drain yielded to the steer");
    assert_eq!(fx.messages[0].ordinal, None, "no turn ran, so no save ran");
    assert!(persisted(&fx).await.is_none());
}

#[tokio::test]
async fn a_drain_that_runs_no_turn_saves_nothing() {
    let mut fx = Fixture::new();
    {
        let mut ctx = fx.ctx();
        super::super::drain_pending_and_nudge(&mut ctx).await;
    }
    assert!(fx.messages.is_empty(), "nothing was pending");
    assert!(
        persisted(&fx).await.is_none(),
        "no turn ran, so no save ran"
    );
}

#[tokio::test]
async fn an_ephemeral_loop_runs_the_turn_and_never_writes_the_store() {
    let mut fx = Fixture::new();
    fx.set_ephemeral(true);
    follow_up(&mut fx, "reply ok").await;
    assert!(!fx.messages.is_empty(), "the turn must have run");
    assert!(
        fx.messages.iter().all(|m| m.ordinal.is_none()),
        "an ephemeral loop has no durable ordinals to serve"
    );
    assert!(persisted(&fx).await.is_none());
}

#[path = "uds_dispatch_report_delivery_tests.rs"]
mod delivery_tests;

#[path = "uds_dispatch_report_recall_tests.rs"]
mod recall_tests;

#[path = "uds_dispatch_report_ledger_tests.rs"]
mod ledger_tests;
