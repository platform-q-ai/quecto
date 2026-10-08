//! #843, #2192: `get_messages` on an ended sub-agent pages the transcript
//! it persisted. A launched child runs as the session `cli:<uuid>` (its
//! `-s <uuid>`), so that is the session read — never one keyed by the bare
//! uuid, which no child writes.
use super::forward_subagent_get_messages;
use crate::application::sessions::ports::SessionStore;
use crate::domain::conversation::value_objects::message::Message;
use crate::domain::sessions::entities::session::{Session, SubagentLiveness};
use crate::domain::sessions::entities::session_identity::SessionIdentity;
use crate::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentRegistry, SubagentStatus, new_registry,
};

use super::tests_843::Fx;

/// A registry holding one ended child, uuid `dead-child`, label `dead-label`.
fn ended_child() -> SubagentRegistry {
    let registry = new_registry();
    let mut entry = SubagentEntry::with_identity(
        crate::domain::ids::AgentUuid::from("dead-child"),
        "dead-label".into(),
        "/tmp/dead.sock".into(),
        0,
    );
    entry.origin = crate::domain::agents::value_objects::child_end::ChildOrigin::Launched;
    entry.status = SubagentStatus::Exited;
    entry.persisted_liveness = SubagentLiveness::Dead;
    registry.lock().unwrap().insert("dead-child".into(), entry);
    registry
}

/// Persist `messages` under `key`, as the child running as it would.
async fn persist(fx: &Fx, key: SessionIdentity, messages: Vec<Message>) {
    fx.store
        .save(&Session {
            key,
            messages,
            workflow_run: None,
            subagent_roster: Vec::new(),
        })
        .await
        .unwrap();
}

fn child_session() -> SessionIdentity {
    SessionIdentity::named_cli("dead-child").unwrap()
}

async fn get_messages(
    fx: &mut Fx,
    agent: &str,
    count: Option<usize>,
    before: Option<&str>,
) -> serde_json::Value {
    let mut ctx = fx.ctx();
    ctx.subagent_registry = Some(ended_child());
    let event = forward_subagent_get_messages(
        &ctx,
        Some(crate::domain::ids::CommandId::from("hist")),
        "get_messages",
        crate::domain::ids::AgentId::from(agent),
        count,
        before.map(crate::domain::ids::MessageId::from),
    )
    .await;
    serde_json::to_value(event).unwrap()
}

#[tokio::test]
async fn forward_get_messages_reads_dead_historical_transcript_by_uuid() {
    let mut fx = Fx::new();
    persist(
        &fx,
        child_session(),
        vec![Message::user("historical transcript")],
    )
    .await;
    let json = get_messages(&mut fx, "dead-child", Some(10), None).await;
    assert_eq!(json["success"], true, "{json}");
    assert!(json.to_string().contains("historical transcript"));
}

#[tokio::test]
async fn forward_get_messages_reads_a_dead_child_by_its_label() {
    let mut fx = Fx::new();
    persist(&fx, child_session(), vec![Message::user("by label")]).await;
    let json = get_messages(&mut fx, "dead-label", None, None).await;
    assert_eq!(json["success"], true, "{json}");
    assert!(json.to_string().contains("by label"));
}

#[tokio::test]
async fn a_session_keyed_by_the_bare_uuid_is_not_the_childs() {
    let mut fx = Fx::new();
    persist(
        &fx,
        SessionIdentity::from_persisted_key("dead-child"),
        vec![Message::user("not the child's")],
    )
    .await;
    let json = get_messages(&mut fx, "dead-child", None, None).await;
    assert_eq!(json["success"], false, "{json}");
    assert_eq!(
        json["error"],
        "no persisted transcript for subagent 'dead-child'"
    );
}

#[tokio::test]
async fn forward_get_messages_reads_full_dead_historical_transcript_when_count_omitted() {
    use crate::interface::cli::uds_session::HISTORY_PAGE_SIZE;
    let mut fx = Fx::new();
    let messages: Vec<_> = (0..(HISTORY_PAGE_SIZE + 3))
        .map(|i| Message::user(format!("historical-{i}")))
        .collect();
    persist(&fx, child_session(), messages).await;
    let json = get_messages(&mut fx, "dead-child", None, None).await;
    let returned = json["data"]["messages"].as_array().unwrap();
    assert_eq!(returned.len(), HISTORY_PAGE_SIZE + 3);
    assert!(json.to_string().contains("historical-0"));
}

#[tokio::test]
async fn forward_get_messages_rejects_stale_historical_before_cursor() {
    let mut fx = Fx::new();
    persist(
        &fx,
        child_session(),
        vec![Message::user("historical transcript")],
    )
    .await;
    let json = get_messages(&mut fx, "dead-child", None, Some("missing-cursor")).await;
    assert_eq!(json["success"], false);
    assert_eq!(json["error"], "history cursor not found: missing-cursor");
}

/// #2192: a row whose uuid names no session a child runs as says so; it is
/// not answered with the route's unrelated refusal.
#[tokio::test]
async fn a_uuid_that_names_no_session_is_refused_for_what_it_is() {
    let mut fx = Fx::new();
    let registry = new_registry();
    let mut entry = SubagentEntry::with_identity(
        crate::domain::ids::AgentUuid::from("has space"),
        "odd".into(),
        "/tmp/odd.sock".into(),
        0,
    );
    entry.origin = crate::domain::agents::value_objects::child_end::ChildOrigin::Launched;
    entry.persisted_liveness = SubagentLiveness::Dead;
    registry.lock().unwrap().insert("has space".into(), entry);
    let mut ctx = fx.ctx();
    ctx.subagent_registry = Some(registry);
    let event = forward_subagent_get_messages(
        &ctx,
        None,
        "get_messages",
        crate::domain::ids::AgentId::from("odd"),
        None,
        None,
    )
    .await;
    let json = serde_json::to_value(event).unwrap();
    assert_eq!(json["success"], false);
    let error = json["error"].as_str().unwrap();
    assert!(
        error.starts_with("subagent 'odd' names no session a child runs as:"),
        "{error}"
    );
}

/// #2192 review round 5 (M2): a "descendant" a child reported and then
/// let be pruned, keyed `secret-plan`, does not make the history fallback
/// read the session `cli:secret-plan`: only a launched child's transcript
/// is read. A launched child named like it is still read.
#[tokio::test]
async fn a_reported_row_does_not_open_another_sessions_transcript() {
    let mut fx = Fx::new();
    let secret = SessionIdentity::named_cli("secret-plan").unwrap();
    persist(&fx, secret, vec![Message::user("THE SECRET PLAN")]).await;
    let registry = ended_child();
    let mut reported = SubagentEntry::with_identity(
        crate::domain::ids::AgentUuid::from("secret-plan"),
        "secret-plan".into(),
        "/tmp/s.sock".into(),
        0,
    );
    reported.origin = crate::domain::agents::value_objects::child_end::ChildOrigin::Reported;
    reported.status = SubagentStatus::Exited;
    reported.persisted_liveness = SubagentLiveness::Dead;
    registry
        .lock()
        .unwrap()
        .insert("secret-plan".into(), reported);
    let mut ctx = fx.ctx();
    ctx.subagent_registry = Some(registry);
    let event = forward_subagent_get_messages(
        &ctx,
        Some(crate::domain::ids::CommandId::from("hist")),
        "get_messages",
        crate::domain::ids::AgentId::from("secret-plan"),
        Some(10),
        None,
    )
    .await;
    let json = serde_json::to_value(event).unwrap();
    assert_eq!(json["success"], false, "{json}");
    assert!(!json.to_string().contains("THE SECRET PLAN"), "{json}");
    assert!(
        json.to_string().contains("not launched by this harness"),
        "{json}"
    );
}
