//! A follower's routes (epic #2705 B11): every write is 409 `standby`
//! naming its primary, reads are served from its replica, and its status
//! says how far behind it is.
use std::sync::Arc;

use serde_json::{Value, json};

use super::tests::{BOARD, operating, serving, written};
use super::*;
use crate::application::project_board::ports::replication::{ReplicaState, ReplicaStatus};
use crate::infrastructure::http::board_service::listener::{
    ServiceMethod, ServiceRequest, Transport,
};
use crate::infrastructure::test_support::memory_board_tokens::MemoryBoardTokens;
use crate::infrastructure::time::board_clock::SystemBoardClock;

const PRIMARY: &str = "http://100.64.0.1:7979";

fn follower() -> BoardRoutes {
    let (store, _) = written();
    let status = ReplicaStatus {
        primary: PRIMARY.into(),
        applied: 2,
        primary_head: Some(5),
        state: ReplicaState::Following,
    };
    let serve = serving(&store).following(Arc::new(move || status.clone()));
    let tokens = Arc::new(MemoryBoardTokens::default());
    let access = BoardAccess::new(tokens, Arc::new(SystemBoardClock::default()));
    BoardRoutes::new(serve, access, operating(&store)).standby(PRIMARY.into())
}

fn sent(routes: &BoardRoutes, method: ServiceMethod, path: &str, body: Value) -> (u16, Value) {
    let reply = routes.answer(&ServiceRequest {
        method,
        path: path.into(),
        body: match body {
            Value::Null => vec![],
            body => body.to_string().into_bytes(),
        },
        transport: Transport::Local,
        principal: None,
        query: None,
        last_event_id: None,
    });
    (reply.status, serde_json::from_slice(&reply.body).unwrap())
}

fn op(routes: &BoardRoutes, request: Value) -> (u16, Value) {
    let body = json!({"request": request, "front_end": "cli",
        "write_id": "6f1c0a52-7d0e-4c55-9a51-2b8f3a3e8c10",
        "committer": {"name": "Ada", "email": "ada@example.com"}});
    let path = format!("/v1/boards/{BOARD}/op");
    sent(routes, ServiceMethod::Post, &path, body)
}

fn standby(answer: &(u16, Value)) {
    assert_eq!(answer.0, 409, "{}", answer.1);
    assert_eq!(answer.1["code"], "standby");
    assert_eq!(answer.1["primary"], PRIMARY);
}

#[test]
fn a_write_to_a_follower_is_409_standby_with_its_primary() {
    let routes = follower();
    standby(&op(
        &routes,
        json!({"op": "create", "task": "b", "title": "B", "status": "ready"}),
    ));
    standby(&op(&routes, json!({"op": "bootstrap"})));
}

#[test]
fn a_follower_serves_reads_from_its_replica() {
    let (status, body) = op(&follower(), json!({"op": "list"}));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["answer"]["ok"], true, "{body}");
}

#[test]
fn a_follower_administers_no_token() {
    let routes = follower();
    let name = json!({"name": "phone"});
    standby(&sent(
        &routes,
        ServiceMethod::Post,
        "/v1/admin/tokens",
        name.clone(),
    ));
    standby(&sent(
        &routes,
        ServiceMethod::Post,
        "/v1/admin/tokens/revoke",
        name,
    ));
    let (status, _) = sent(&routes, ServiceMethod::Get, "/v1/admin/tokens", Value::Null);
    assert_eq!(status, 200, "its primary's tokens are listed");
}

#[test]
fn a_followers_status_shows_its_lag() {
    let (status, body) = sent(&follower(), ServiceMethod::Get, "/v1/status", Value::Null);
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["role"], "follower");
    assert_eq!(
        body["replication"],
        json!({"primary": PRIMARY, "applied": 2, "primary_head": 5,
               "state": "following", "reason": null})
    );
}
