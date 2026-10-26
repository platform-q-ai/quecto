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
    follower_in(ReplicaState::Following, true)
}

/// A follower whose replication is in `state`, serving devices or not.
fn follower_in(state: ReplicaState, devices: bool) -> BoardRoutes {
    let (store, _) = written();
    let status = ReplicaStatus {
        primary: PRIMARY.into(),
        applied: 2,
        primary_head: Some(5),
        state,
    };
    let serve = serving(&store)
        .following(Arc::new(move || status.clone()))
        .serving_devices(Arc::new(move || devices));
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

/// A device's read of the status over TCP: its status and code.
fn device_reads(routes: &BoardRoutes) -> (u16, Value) {
    let reply = routes.answer(&device(ServiceMethod::Get, "/v1/status", vec![]));
    let body: Value = serde_json::from_slice(&reply.body).unwrap();
    (reply.status, body["code"].clone())
}

/// A device's request over TCP.
fn device(method: ServiceMethod, path: &str, body: Vec<u8>) -> ServiceRequest {
    ServiceRequest {
        method,
        path: path.into(),
        body,
        transport: Transport::Tcp,
        principal: Some("phone".into()),
        query: None,
        last_event_id: None,
    }
}

/// B11 review 2: a follower whose copy is not kept current (stalled past
/// its bound, or diverged) refuses devices' reads, so a token its primary
/// revoked meanwhile reads nothing; the Unix socket is served still.
#[test]
fn a_stale_follower_refuses_devices_reads_and_serves_the_socket() {
    let stalled = || ReplicaState::Stalled("no primary".into());
    let short = follower_in(stalled(), true);
    assert_eq!(device_reads(&short).0, 200, "stalled briefly, it serves");
    let refusal = (503, json!("standby_stale"));
    for stale in [
        follower_in(stalled(), false),
        follower_in(ReplicaState::Diverged("forked".into()), false),
    ] {
        assert_eq!(device_reads(&stale), refusal);
        let (status, _) = sent(&stale, ServiceMethod::Get, "/v1/status", Value::Null);
        assert_eq!(status, 200, "the socket is served");
    }
}

/// B11 review 4: the change feed is refused to devices too, and a
/// device's write to a stale follower is told its primary.
#[test]
fn a_stale_followers_feed_and_writes_are_refused_naming_its_primary() {
    use crate::infrastructure::http::board_service::listener::ServiceHandler;
    let stale = follower_in(ReplicaState::Stalled("no primary".into()), false);
    let events = format!("/v1/boards/{BOARD}/events");
    let wake: StreamWake = Arc::new(|| {});
    let refused = stale.stream(&device(ServiceMethod::Get, &events, vec![]), wake);
    let Err(reply) = refused else {
        panic!("the feed is refused to a device");
    };
    let body: Value = serde_json::from_slice(&reply.body).unwrap();
    assert_eq!(
        (reply.status, &body["code"]),
        (503, &json!("standby_stale"))
    );
    assert_eq!(body["primary"], PRIMARY);
    let create = json!({"request": {"op": "create", "task": "b", "title": "B", "status": "ready"},
        "front_end": "cli", "write_id": "6f1c0a52-7d0e-4c55-9a51-2b8f3a3e8c10",
        "committer": {"name": "Ada", "email": "ada@example.com"}});
    let op = format!("/v1/boards/{BOARD}/op");
    let reply = stale.answer(&device(
        ServiceMethod::Post,
        &op,
        create.to_string().into_bytes(),
    ));
    let body: Value = serde_json::from_slice(&reply.body).unwrap();
    assert_eq!(body["primary"], PRIMARY, "{body}");
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
