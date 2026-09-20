//! The picker path and the action routes of `resume_session` through the real
//! dispatch over the production composition (#2011 review R1-H1, R1-H8).
use super::fixture_tests::Fixture;
use crate::application::sessions::ports::SessionStore;
use crate::domain::message::Message;
use crate::domain::session::Session;
use crate::domain::session_identity::SessionIdentity;
use crate::interface::cli::protocol::AgentCommand;

/// Dispatch one wire line and return the answer with the same `id`.
async fn answer(fx: &mut Fixture, line: serde_json::Value) -> serde_json::Value {
    let id = line["id"].as_str().unwrap().to_string();
    let command: AgentCommand = serde_json::from_value(line).expect("a protocol command");
    let (tx, mut rx) = tokio::sync::broadcast::channel(64);
    let mut ctx = fx.ctx();
    ctx.broadcast_tx = Some(tx);
    assert!(!super::dispatch_command(command, &mut ctx).await);
    std::iter::from_fn(|| rx.try_recv().ok())
        .map(|frame| serde_json::from_str::<serde_json::Value>(&frame).unwrap())
        .find(|event| event["id"] == id.as_str())
        .expect("the correlated answer")
}

async fn saved_here(fx: &Fixture, key: &str) {
    crate::interface::cli::uds::dispatch_session_roster_tests::seed_home(&fx.store, key).await;
    let mut session = Session::new(SessionIdentity::from_persisted_key(key));
    session.messages.push(Message::user("saved history"));
    fx.store.save(&session).await.unwrap();
    fx.store.release(&session.key);
}

/// The primary picker path, on its positive side: the version a listed row
/// carries is the token the resume transaction accepts.
#[tokio::test]
async fn the_version_a_listed_row_carries_restores_that_row() {
    let mut fx = Fixture::new();
    saved_here(&fx, "cli:picked").await;
    saved_here(&fx, "cli:other").await;
    let list = serde_json::json!({"type": "list_sessions", "id": "l1", "scope": "local"});
    let listed = answer(&mut fx, list).await;
    let rows = listed["data"]["sessions"].as_array().unwrap();
    let row = rows.iter().find(|row| row["key"] == "cli:picked").unwrap();
    assert_eq!(row["resumeEligible"], true, "{row}");
    let other = rows.iter().find(|row| row["key"] == "cli:other").unwrap();
    assert_ne!(row["homeVersion"], other["homeVersion"], "one per session");
    // Another row's version authorizes nothing for this one.
    let wrong = serde_json::json!({
        "type": "resume_session", "id": "r0", "session": "cli:picked",
        "expectedHomeVersion": other["homeVersion"],
    });
    let refused = answer(&mut fx, wrong).await;
    assert_eq!(refused["data"]["code"], "stale_home_version", "{refused}");
    let pick = serde_json::json!({
        "type": "resume_session", "id": "r1", "session": row["key"],
        "expectedHomeVersion": row["homeVersion"],
    });
    let resumed = answer(&mut fx, pick).await;
    assert_eq!(resumed["success"], true, "{resumed}");
    assert_eq!(resumed["data"]["outcome"], "resumed");
    assert_eq!(resumed["data"]["sessionKey"], "cli:picked");
    assert_eq!(fx.current_session_key(), "cli:picked");
}
