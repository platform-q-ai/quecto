//! The picker path and the action routes of `resume_session` through the real
//! dispatch over the production composition (#2011 review R1-H1, R1-H8).
use super::fixture_tests::Fixture;
use crate::application::sessions::dto::ActionAvailability;
use crate::application::sessions::ports::SessionStore;
use crate::domain::message::Message;
use crate::domain::resume_decision::ResumeAction;
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

/// A saved session whose home is the folder `dir` (or none: a legacy record).
async fn saved_at(fx: &Fixture, key: &str, dir: Option<&std::path::Path>) {
    let mut session = Session::new(SessionIdentity::from_persisted_key(key));
    // The home is recorded before the first save, as production does.
    if let Some(dir) = dir {
        let home = crate::domain::session_home::SessionHome {
            execution_dir: dir.to_path_buf(),
            group: crate::domain::session_home::WorkspaceGroup::Folder {
                directory: dir.to_path_buf(),
            },
            provenance: crate::domain::session_home::AssociationProvenance::SavedHere,
        };
        fx.store.record_new_home(&session.key, &home).unwrap();
    }
    session.messages.push(Message::user("saved history"));
    fx.store.save(&session).await.unwrap();
    fx.store.release(&session.key);
}

/// The hand-off contract of the executor slices (#2012–#2014): an action the
/// production composition declares executable must be routed by this dispatch
/// to its own transaction. Otherwise a selectable dialog row would be answered
/// `action_executed_elsewhere` ("is not a restore") — so the capability flip
/// cannot land without its route. Each action is asked of a session whose
/// decision OFFERS it (review R2-H3: an action the kind does not offer is
/// `action_not_offered` whatever is composed). An action that is not composed
/// is refused `action_unavailable`, never substituted.
#[tokio::test]
async fn every_composed_capability_has_a_dispatch_route() {
    let composed = crate::composition::resume_capabilities::composed();
    let mut fx = Fixture::new();
    let elsewhere = tempfile::tempdir().unwrap();
    let gone = elsewhere.path().join("gone");
    saved_at(&fx, "cli:elsewhere", Some(elsewhere.path())).await;
    saved_at(&fx, "cli:gone", Some(&gone)).await;
    saved_at(&fx, "cli:legacy", None).await;
    for action in ResumeAction::ALL {
        let session = match action {
            ResumeAction::OpenOriginal | ResumeAction::ForkCurrent => "cli:elsewhere",
            ResumeAction::Locate => "cli:gone",
            ResumeAction::Associate => "cli:legacy",
            ResumeAction::Cancel => continue,
        };
        let ask = serde_json::json!({"type": "resume_session", "id": "d1", "session": session});
        let decision = answer(&mut fx, ask).await;
        let offered = decision["data"]["actions"].as_array().expect("a decision");
        assert!(
            offered.iter().any(|offer| offer["action"] == action.name()),
            "{action:?}: {decision}"
        );
        let request = serde_json::json!({
            "type": "resume_session", "id": "a1", "session": session,
            "action": action.name(), "expectedHomeVersion": decision["data"]["homeVersion"],
        });
        let answered = answer(&mut fx, request).await;
        let code = answered["data"]["code"].as_str().unwrap_or_default();
        match composed.availability(action) {
            ActionAvailability::Available => assert_ne!(
                code, "action_executed_elsewhere",
                "{action:?} is composed executable but the dispatch has no route for it"
            ),
            ActionAvailability::Unavailable(_) => {
                assert_eq!(code, "action_unavailable", "{action:?}: {answered}");
                assert_eq!(answered["data"]["action"], action.name());
            }
        }
    }
}

/// The documented order on the wire (review R2-H3): a missing session is
/// `not_found` whatever action and token come with it; an action its kind does
/// not offer is `action_not_offered` and names the action.
#[tokio::test]
async fn an_action_is_answered_in_the_documented_order() {
    let mut fx = Fixture::new();
    saved_here(&fx, "cli:here").await;
    let list = serde_json::json!({"type": "list_sessions", "id": "l1", "scope": "global"});
    let listed = answer(&mut fx, list).await;
    let version = listed["data"]["sessions"][0]["homeVersion"].clone();
    let ask = |session: &str, version: &serde_json::Value| {
        serde_json::json!({
            "type": "resume_session", "id": "o1", "session": session,
            "action": "locate", "expectedHomeVersion": version,
        })
    };
    let missing = answer(&mut fx, ask("nope", &version)).await;
    assert_eq!(missing["data"]["code"], "not_found", "{missing}");
    let stale = answer(&mut fx, ask("cli:here", &"h1-0000000000000000".into())).await;
    assert_eq!(stale["data"]["code"], "stale_home_version", "{stale}");
    let unoffered = answer(&mut fx, ask("cli:here", &version)).await;
    assert_eq!(
        unoffered["data"]["code"], "action_not_offered",
        "{unoffered}"
    );
    assert_eq!(unoffered["data"]["action"], "locate");
    assert_eq!(fx.current_session_key(), "cli:test", "nothing was restored");
}
