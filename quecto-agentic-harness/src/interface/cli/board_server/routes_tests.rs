use super::*;
use crate::application::project_board::ports::{BoardArtifact, BoardHead, ProjectBoardStore};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::BoardId;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};
use crate::infrastructure::http::board_service::listener::{
    ServiceMethod, ServiceRequest, Transport,
};
use crate::infrastructure::test_support::board_artifacts::plan;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::test_support::memory_board_tokens::MemoryBoardTokens;
use crate::infrastructure::time::board_clock::SystemBoardClock;
use serde_json::Value;
use std::sync::Arc;

pub(in crate::interface::cli::board_server) const BOARD: &str = "0123456789abcdef";

fn task(id: &str) -> Task {
    let at = Timestamp::parse("2026-10-24T08:00:00Z").unwrap();
    Task::new(TaskFields {
        id: Slug::parse(id).unwrap(),
        title: format!("Task {id}"),
        kind: TaskKind::Task,
        description: "Do it.".into(),
        description_form: Default::default(),
        status: TaskStatus::Ready,
        parent: None,
        depends_on: vec![],
        accepted_plan: None,
        claim: None,
        prs: vec![],
        github_issue: None,
        resolution: None,
        superseded_by: vec![],
        references: vec![],
        created: at.clone(),
        updated: at,
        reason: None,
    })
    .unwrap()
}

/// The server's reads of `store`'s board, served as board [`BOARD`].
pub(in crate::interface::cli::board_server) fn serving(store: &MemoryBoardStore) -> ServeBoard {
    ServeBoard::new(
        Arc::new(store.client()),
        Arc::new(SystemBoardClock::default()),
        BoardId::of_hex(BOARD).unwrap(),
    )
}

/// The routes over `serve`, with access over in-memory tokens.
pub(in crate::interface::cli::board_server) fn routes_of(serve: ServeBoard) -> BoardRoutes {
    routes_with(serve, Arc::new(MemoryBoardTokens::default()))
}

fn routes_with(serve: ServeBoard, tokens: Arc<MemoryBoardTokens>) -> BoardRoutes {
    let access = BoardAccess::new(tokens, Arc::new(SystemBoardClock::default()));
    BoardRoutes::new(serve, access)
}

/// A board holding a task and a plan of it.
pub(in crate::interface::cli::board_server) fn written() -> (MemoryBoardStore, BoardHead) {
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    let head = store.read().unwrap().head;
    let artifacts = [
        BoardArtifact::Task(task("alpha")),
        BoardArtifact::Plan(plan("alpha", 1)),
    ];
    let written = store.write(&head, &artifacts, "seed").unwrap();
    let crate::application::project_board::ports::BoardWrite::Written(head) = written else {
        panic!("the seed landed: {written:?}");
    };
    (store, head)
}

fn get(routes: &BoardRoutes, path: &str) -> (u16, Value) {
    answer(routes, ServiceMethod::Get, path)
}

fn answer(routes: &BoardRoutes, method: ServiceMethod, path: &str) -> (u16, Value) {
    let reply = routes.answer(&ServiceRequest {
        method,
        path: path.into(),
        body: vec![],
        transport: Transport::Local,
        principal: None,
    });
    let body = serde_json::from_slice(&reply.body)
        .unwrap_or_else(|error| panic!("{path}: {error}: {:?}", reply.body));
    (reply.status, body)
}

#[test]
fn status_answers_the_role_the_board_and_the_head() {
    let (store, head) = written();
    let routes = routes_of(serving(&store));
    let (status, body) = get(&routes, "/v1/status");
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["role"], "primary");
    assert_eq!(body["board"], BOARD);
    assert_eq!(body["head"], head.0.as_str());
}

#[test]
fn status_of_no_board_answers_no_head() {
    let routes = routes_of(serving(&MemoryBoardStore::default()));
    let (status, body) = get(&routes, "/v1/status");
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["head"], Value::Null);
}

#[test]
fn time_answers_the_servers_clock() {
    let (store, _) = written();
    let (status, body) = get(&routes_of(serving(&store)), "/v1/time");
    assert_eq!(status, 200, "{body}");
    let time = body["time"].as_str().unwrap();
    assert!(Timestamp::parse(time).is_ok(), "{time}");
}

/// A route's 405 names the methods it takes, with its code.
#[test]
fn a_routes_405_names_its_methods() {
    let (store, _) = written();
    let routes = routes_of(serving(&store));
    for path in ["/v1/status", "/v1/time"] {
        let reply = routes.answer(&ServiceRequest {
            method: ServiceMethod::Post,
            path: path.into(),
            body: vec![],
            transport: Transport::Local,
            principal: None,
        });
        assert_eq!((reply.status, reply.allow), (405, Some("GET")), "{path}");
        assert_eq!(
            answer(&routes, ServiceMethod::Post, path).1["code"],
            "method"
        );
    }
    assert_eq!(routes.allowed("/v1/time"), Some("GET"));
    assert_eq!(routes.allowed("/v1/nothing"), None);
}

/// The snapshot route moved to B11: its path is no route here.
#[test]
fn a_path_outside_the_route_table_is_404() {
    let (store, _) = written();
    let routes = routes_of(serving(&store));
    for path in [
        "/",
        "/v1",
        "/v1/status/",
        "/v2/status",
        "/v1/boards",
        &format!("/v1/boards/{BOARD}/snapshot"),
    ] {
        let (status, body) = get(&routes, path);
        assert_eq!(
            (status, &body["code"]),
            (404, &Value::from("route")),
            "{path}"
        );
    }
}

#[test]
fn a_store_that_cannot_be_read_is_503() {
    let (store, _) = written();
    store.fail_every_call(
        crate::application::project_board::ports::BoardStoreError::Unavailable(
            "the remote is down".into(),
        ),
    );
    let routes = routes_of(serving(&store));
    let (status, body) = get(&routes, "/v1/status");
    assert_eq!((status, &body["code"]), (503, &Value::from("unavailable")));
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("the remote is down")
    );
}

/// A request as `transport` carries it, its reply and body.
fn sent(
    routes: &BoardRoutes,
    transport: Transport,
    authorization: Option<&str>,
    method: ServiceMethod,
    path: &str,
    body: &str,
) -> (
    crate::infrastructure::http::board_service::listener::ServiceReply,
    Value,
) {
    // Over TCP the listener asks the gate first, as it does live.
    let principal = match transport {
        Transport::Tcp => match routes.authenticate_remote(authorization) {
            Ok(principal) => Some(principal),
            Err(reply) => {
                let body = serde_json::from_slice(&reply.body).unwrap();
                return (reply, body);
            }
        },
        Transport::Local => None,
    };
    let reply = routes.answer(&ServiceRequest {
        method,
        path: path.into(),
        body: body.as_bytes().to_vec(),
        transport,
        principal,
    });
    let body = serde_json::from_slice(&reply.body).unwrap();
    (reply, body)
}

/// B5 red: over TCP, a request with no token, an unknown one, one not
/// shaped `Bearer <token>`, or a revoked one is 401 `token` (on any path,
/// before routing); a held token reads.
#[test]
fn tcp_reads_need_a_held_bearer_token() {
    let (store, _) = written();
    let tokens = Arc::new(MemoryBoardTokens::default());
    let routes = routes_with(serving(&store), Arc::clone(&tokens));
    let (_, added) = sent(
        &routes,
        Transport::Local,
        None,
        ServiceMethod::Post,
        "/v1/admin/tokens",
        r#"{"name":"laptop"}"#,
    );
    let token = added["token"].as_str().unwrap().to_string();
    let bearer = format!("Bearer {token}");
    for refused in [
        None,
        Some("Bearer token-phone"),
        Some(token.as_str()),
        Some("Basic dXNlcjpwYXNz"),
        Some("Bearer "),
        Some(""),
    ] {
        for path in ["/v1/status", "/v1/nothing", "/v1/admin/tokens"] {
            let (reply, body) = sent(
                &routes,
                Transport::Tcp,
                refused,
                ServiceMethod::Get,
                path,
                "",
            );
            assert_eq!(
                (reply.status, &body["code"]),
                (401, &Value::from("token")),
                "{refused:?} {path}"
            );
            assert_eq!(reply.challenge, Some("Bearer"));
        }
    }
    for scheme in ["Bearer", "bearer", "BEARER"] {
        let authorization = format!("{scheme} {token}");
        let (reply, body) = sent(
            &routes,
            Transport::Tcp,
            Some(&authorization),
            ServiceMethod::Get,
            "/v1/status",
            "",
        );
        assert_eq!(reply.status, 200, "{body}");
        assert_eq!(body["board"], BOARD);
    }
    let (reply, _) = sent(
        &routes,
        Transport::Tcp,
        Some(&bearer),
        ServiceMethod::Get,
        "/v1/time",
        "",
    );
    assert_eq!(reply.status, 200);
    let (reply, _) = sent(
        &routes,
        Transport::Local,
        None,
        ServiceMethod::Post,
        "/v1/admin/tokens/revoke",
        r#"{"name":"laptop"}"#,
    );
    assert_eq!(reply.status, 200);
    let (reply, body) = sent(
        &routes,
        Transport::Tcp,
        Some(&bearer),
        ServiceMethod::Get,
        "/v1/status",
        "",
    );
    assert_eq!((reply.status, &body["code"]), (401, &Value::from("token")));
}

/// B5 red: admin routes over TCP are 403 `local_only`, even with a held
/// token; nothing is changed.
#[test]
fn admin_over_tcp_is_403_even_with_a_held_token() {
    let (store, _) = written();
    let routes = routes_of(serving(&store));
    let (_, added) = sent(
        &routes,
        Transport::Local,
        None,
        ServiceMethod::Post,
        "/v1/admin/tokens",
        r#"{"name":"laptop"}"#,
    );
    let bearer = format!("Bearer {}", added["token"].as_str().unwrap());
    for (method, path, body) in [
        (ServiceMethod::Get, "/v1/admin/tokens", ""),
        (
            ServiceMethod::Post,
            "/v1/admin/tokens",
            r#"{"name":"phone"}"#,
        ),
        (
            ServiceMethod::Post,
            "/v1/admin/tokens/revoke",
            r#"{"name":"laptop"}"#,
        ),
    ] {
        let (reply, answer) = sent(&routes, Transport::Tcp, Some(&bearer), method, path, body);
        assert_eq!(
            (reply.status, &answer["code"]),
            (403, &Value::from("local_only")),
            "{path}"
        );
    }
    let (_, listed) = sent(
        &routes,
        Transport::Local,
        None,
        ServiceMethod::Get,
        "/v1/admin/tokens",
        "",
    );
    assert_eq!(listed["tokens"].as_array().unwrap().len(), 1, "{listed}");
    let (reply, _) = sent(
        &routes,
        Transport::Tcp,
        Some(&bearer),
        ServiceMethod::Get,
        "/v1/status",
        "",
    );
    assert_eq!(reply.status, 200, "the token still reads");
}

/// Over the socket the admin routes add (201, the token once), list
/// (names and dates, never a token) and revoke; each failure is coded.
#[test]
fn admin_routes_over_the_socket_add_list_and_revoke() {
    let (store, _) = written();
    let routes = routes_of(serving(&store));
    let local = |method, path, body| sent(&routes, Transport::Local, None, method, path, body);
    let (reply, added) = local(
        ServiceMethod::Post,
        "/v1/admin/tokens",
        r#"{"name":"laptop"}"#,
    );
    assert_eq!(reply.status, 201, "{added}");
    assert_eq!(added["name"], "laptop");
    let token = added["token"].as_str().unwrap().to_string();
    assert!(Timestamp::parse(added["created"].as_str().unwrap()).is_ok());
    let (reply, listed) = local(ServiceMethod::Get, "/v1/admin/tokens", "");
    assert_eq!(reply.status, 200);
    assert_eq!(listed["tokens"][0]["name"], "laptop");
    assert_eq!(listed["tokens"][0]["created"], added["created"]);
    assert!(!listed.to_string().contains(&token), "{listed}");
    for (body, status, code) in [
        (r#"{"name":"laptop"}"#, 409, "token_exists"),
        (r#"{"name":"Laptop"}"#, 400, "token_name"),
        (r#"{"name":"x","extra":1}"#, 400, "request"),
        ("not json", 400, "request"),
        ("", 400, "request"),
    ] {
        let (reply, answer) = local(ServiceMethod::Post, "/v1/admin/tokens", body);
        assert_eq!(
            (reply.status, &answer["code"]),
            (status, &Value::from(code)),
            "{body}"
        );
    }
    let (reply, answer) = local(
        ServiceMethod::Post,
        "/v1/admin/tokens/revoke",
        r#"{"name":"phone"}"#,
    );
    assert_eq!(
        (reply.status, &answer["code"]),
        (404, &Value::from("no_token"))
    );
    let (reply, answer) = local(
        ServiceMethod::Post,
        "/v1/admin/tokens/revoke",
        r#"{"name":"laptop"}"#,
    );
    assert_eq!(
        (reply.status, &answer["revoked"]),
        (200, &Value::from(true))
    );
    let (reply, answer) = local(ServiceMethod::Get, "/v1/admin/tokens/revoke", "");
    assert_eq!(
        (reply.status, reply.allow, &answer["code"]),
        (405, Some("POST"), &Value::from("method"))
    );
    assert_eq!(routes.allowed("/v1/admin/tokens"), Some("GET, POST"));
}

#[test]
fn tokens_that_cannot_be_read_are_503() {
    let (store, _) = written();
    let tokens = Arc::new(MemoryBoardTokens::default());
    let routes = routes_with(serving(&store), Arc::clone(&tokens));
    tokens.fail_with(
        crate::application::project_board::ports::TokenError::Unavailable(
            "the disk is gone".into(),
        ),
    );
    let (reply, body) = sent(
        &routes,
        Transport::Tcp,
        Some("Bearer x"),
        ServiceMethod::Get,
        "/v1/status",
        "",
    );
    // Review round 1: a TCP caller is never told why (paths, user, board).
    let generic =
        serde_json::json!({"error": "the token store is unavailable", "code": "unavailable"});
    assert_eq!((reply.status, &body), (503, &generic));
    let gated = routes.authenticate_remote(Some("Bearer x")).unwrap_err();
    assert_eq!(gated.status, 503);
    assert_eq!(
        serde_json::from_slice::<Value>(&gated.body).unwrap(),
        generic
    );
    let (reply, body) = sent(
        &routes,
        Transport::Local,
        None,
        ServiceMethod::Get,
        "/v1/admin/tokens",
        "",
    );
    assert_eq!(
        (reply.status, &body["code"]),
        (503, &Value::from("unavailable"))
    );
    assert_eq!(
        body["error"], "the disk is gone",
        "the socket keeps the detail"
    );
    let (reply, _) = sent(
        &routes,
        Transport::Local,
        None,
        ServiceMethod::Get,
        "/v1/status",
        "",
    );
    assert_eq!(reply.status, 200, "a local read needs no token");
}

/// Review round 1: the TCP gate the listener asks first is 401 for no
/// held bearer token, and lets a held one on.
#[test]
fn the_tcp_gate_lets_on_only_a_held_token() {
    let (store, _) = written();
    let routes = routes_of(serving(&store));
    let (_, added) = sent(
        &routes,
        Transport::Local,
        None,
        ServiceMethod::Post,
        "/v1/admin/tokens",
        r#"{"name":"laptop"}"#,
    );
    let bearer = format!("Bearer {}", added["token"].as_str().unwrap());
    assert_eq!(
        routes.authenticate_remote(Some(&bearer)),
        Ok("laptop".into())
    );
    for refused in [None, Some("Bearer token-phone"), Some(""), Some("Basic x")] {
        let reply = routes.authenticate_remote(refused).unwrap_err();
        assert_eq!(
            (reply.status, reply.challenge),
            (401, Some("Bearer")),
            "{refused:?}"
        );
    }
}

/// Review rounds 2 and 3: while the token store is down, its stderr line
/// is written only when the failure changes.
#[test]
fn the_unavailable_line_is_written_on_change_only() {
    let mut last = String::new();
    assert!(
        changed(&mut last, "the disk is gone"),
        "the first is written"
    );
    for _ in 0..100 {
        assert!(!changed(&mut last, "the disk is gone"));
    }
    assert!(changed(&mut last, "must be mode 0600"), "a new failure is");
    assert!(changed(&mut last, "the disk is gone"));
}

/// Review round 3: `answer` trusts the principal the gate handed over:
/// a TCP request without one, or with a name no token has, is 401.
#[test]
fn answer_takes_the_gates_principal() {
    let (store, _) = written();
    let routes = routes_of(serving(&store));
    for principal in [None, Some("Not A Slug".to_string())] {
        let reply = routes.answer(&ServiceRequest {
            method: ServiceMethod::Get,
            path: "/v1/status".into(),
            body: vec![],
            transport: Transport::Tcp,
            principal: principal.clone(),
        });
        assert_eq!(reply.status, 401, "{principal:?}");
    }
    let reply = routes.answer(&ServiceRequest {
        method: ServiceMethod::Get,
        path: "/v1/status".into(),
        body: vec![],
        transport: Transport::Tcp,
        principal: Some("laptop".into()),
    });
    assert_eq!(reply.status, 200);
}
