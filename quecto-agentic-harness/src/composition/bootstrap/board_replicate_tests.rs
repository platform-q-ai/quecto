//! A primary's `replicate` route (epic #2705 B11) over its SQLite board, as
//! composition builds it: a batch the wire's codec reads whole, with the
//! board's revisions, chains and admin rows; a query not as the route
//! takes it is refused by code.
use serde_json::{Value, json};

use super::tests::{Synced, environment, seeded};
use super::*;
use crate::infrastructure::http::board_replica::decode_batch;
use crate::infrastructure::http::board_service::listener::{
    ServiceHandler, ServiceMethod, ServiceRequest, Transport,
};
use crate::interface::cli::board_server::routes::BoardRoutes;

fn asked(
    routes: &BoardRoutes,
    method: ServiceMethod,
    path: &str,
    query: Option<&str>,
    body: Value,
) -> (u16, Vec<u8>) {
    let reply = routes.answer(&ServiceRequest {
        method,
        path: path.into(),
        body: body.to_string().into_bytes(),
        transport: Transport::Local,
        principal: None,
        query: query.map(String::from),
        last_event_id: None,
    });
    (reply.status, reply.body)
}

#[test]
fn the_replicate_route_gives_a_batch_with_its_chains_and_admin_rows() {
    let (dir, work) = seeded();
    let handles =
        build_board_server_with(&environment(&dir, &work, "origin"), Arc::new(Synced)).unwrap();
    let board = handles.serve.board().clone();
    let kind = crate::application::project_board::ports::TokenKind::Replica;
    handles.access.add_kind("eccup", kind).unwrap();
    handles.access.add("phone").unwrap();
    let routes = BoardRoutes::new(handles.serve, handles.access, handles.operations)
        .with_replication(handles.replication);
    let op = format!("/v1/boards/{}/op", board.as_str());
    let bootstrap = json!({"request": {"op": "bootstrap"}, "front_end": "cli",
        "write_id": uuid::Uuid::new_v4().to_string(),
        "committer": {"name": "Ada", "email": "ada@example.com"}});
    let (status, _) = asked(&routes, ServiceMethod::Post, &op, None, bootstrap);
    assert_eq!(status, 200);
    let path = format!("/v1/boards/{}/replicate", board.as_str());
    let (status, body) = asked(
        &routes,
        ServiceMethod::Get,
        &path,
        Some("after=0&objects=0"),
        Value::Null,
    );
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    let batch = decode_batch(&body).expect("the codec reads it");
    assert_eq!((batch.after, batch.tip, batch.more), (0, 1, false));
    assert_eq!(batch.revisions.len(), 1);
    assert_eq!(batch.revisions[0].chain.len(), 64);
    assert!(!batch.objects.is_empty());
    let names: Vec<&str> = batch
        .tokens
        .iter()
        .map(|token| token.name.as_str())
        .collect();
    assert_eq!(names, ["eccup", "phone"]);
    let kinds: Vec<&str> = batch
        .tokens
        .iter()
        .map(|token| token.kind.as_str())
        .collect();
    assert_eq!(
        kinds,
        ["replica", "device"],
        "a follower keeps each token's kind"
    );
    for (query, code) in [
        (Some("after=0"), "query"),
        (Some("objects=0"), "query"),
        (Some("after=0&objects=0&limit=0"), "limit"),
        (Some("after=0&objects=0&since=1"), "query"),
        (None, "query"),
    ] {
        let (status, body) = asked(&routes, ServiceMethod::Get, &path, query, Value::Null);
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            (status, body["code"].as_str()),
            (400, Some(code)),
            "{query:?}"
        );
    }
    // B11 review: over TCP, a device token is refused the batches; a
    // replica token has them.
    let tcp = |principal: &str| {
        let reply = routes.answer(&ServiceRequest {
            method: ServiceMethod::Get,
            path: path.clone(),
            body: vec![],
            transport: Transport::Tcp,
            principal: Some(principal.into()),
            query: Some("after=0&objects=0".into()),
            last_event_id: None,
        });
        let code = serde_json::from_slice::<Value>(&reply.body)
            .ok()
            .and_then(|body| body["code"].as_str().map(String::from));
        (reply.status, code)
    };
    assert_eq!(tcp("phone"), (403, Some("replica_only".into())));
    assert_eq!(tcp("replica:eccup").0, 200);
    let elsewhere = "/v1/boards/0000000000000000/replicate";
    let (status, _) = asked(
        &routes,
        ServiceMethod::Get,
        elsewhere,
        Some("after=0&objects=0"),
        Value::Null,
    );
    assert_eq!(status, 404);
}
