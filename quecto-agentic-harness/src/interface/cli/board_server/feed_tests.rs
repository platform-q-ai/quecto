//! The change feed's routes (epic #2705 B10): the event schema is an
//! allowlist, a batch is at most 500, and a stream opens from its cursor.
use std::sync::{Arc, Mutex};

use super::*;
use crate::application::project_board::ports::BoardStoreError;
use crate::application::project_board::ports::feed::{
    BoardFeed, BoardRevision, FeedDrained, FeedSubscription, FeedTip,
};
use crate::infrastructure::http::board_service::listener::{
    ServiceHandler, ServiceMethod, Transport,
};
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::interface::cli::board_server::routes::BoardRoutes;
use crate::interface::cli::board_server::routes::tests::{BOARD, routes_of, serving};

/// A board of `tip` revisions, all of epoch 1; it records each batch's
/// limit asked.
struct Revisions {
    tip: u64,
    limits: Mutex<Vec<usize>>,
}

struct Quiet;

impl FeedSubscription for Quiet {
    fn drain(&mut self) -> FeedDrained {
        FeedDrained::Revisions(vec![])
    }
}

impl BoardFeed for Revisions {
    fn tip(&self) -> Result<FeedTip, BoardStoreError> {
        Ok(FeedTip {
            epoch: 1,
            revision: self.tip,
        })
    }

    fn revisions(&self, after: u64, limit: BatchLimit) -> Result<RevisionBatch, BoardStoreError> {
        self.limits.lock().unwrap().push(limit.get());
        let revisions = (after + 1..=self.tip)
            .take(limit.get())
            .map(|revision| BoardRevision { epoch: 1, revision })
            .collect();
        Ok(RevisionBatch {
            revisions,
            tip: self.tip()?,
        })
    }

    fn subscribe(&self, _: FeedWake) -> Box<dyn FeedSubscription> {
        Box::new(Quiet)
    }
}

/// A change feed of `tip` revisions, for another module's routes.
pub(in crate::interface::cli::board_server) fn feed_of(tip: u64) -> FollowBoard {
    let feed = Revisions {
        tip,
        limits: Mutex::default(),
    };
    FollowBoard::new(Arc::new(feed) as Arc<dyn BoardFeed>)
}

fn routes(tip: u64) -> (BoardRoutes, Arc<Revisions>) {
    let feed = Arc::new(Revisions {
        tip,
        limits: Mutex::default(),
    });
    let follow = FollowBoard::new(Arc::clone(&feed) as Arc<dyn BoardFeed>);
    let routes = routes_of(serving(&MemoryBoardStore::default())).with_feed(Some(follow));
    (routes, feed)
}

fn request(path: &str, query: Option<&str>, last_event_id: Option<&str>) -> ServiceRequest {
    ServiceRequest {
        method: ServiceMethod::Get,
        path: path.into(),
        body: vec![],
        transport: Transport::Local,
        principal: None,
        query: query.map(str::to_string),
        last_event_id: last_event_id.map(str::to_string),
    }
}

fn revisions_at(routes: &BoardRoutes, query: &str) -> (u16, Value) {
    let path = format!("/v1/boards/{BOARD}/revisions");
    let reply = routes.answer(&request(&path, Some(query), None));
    (reply.status, serde_json::from_slice(&reply.body).unwrap())
}

/// The keys each kind's data may hold: nothing that could carry text.
const SCHEMA: [(&str, &[&str]); 3] = [
    ("epoch", &["epoch", "kind"]),
    ("revision", &["epoch", "kind", "revision"]),
    ("reset", &["after", "kind"]),
];

/// Whether `data` is an event of [`SCHEMA`]: its kind's keys exactly, each
/// but `kind` a number.
fn allowlisted(data: &Value) -> bool {
    let Some(fields) = data.as_object() else {
        return false;
    };
    let kind = fields.get("kind").and_then(Value::as_str);
    SCHEMA.iter().any(|(named, keys)| {
        let mut held: Vec<&str> = fields.keys().map(String::as_str).collect();
        held.sort_unstable();
        Some(*named) == kind
            && held == *keys
            && fields
                .iter()
                .all(|(key, value)| key == "kind" || value.is_u64())
    })
}

/// Red test (plan B10): events carry no artifact text. Every event, and
/// every revision in a batch, is of the allowlisted schema alone.
#[test]
fn every_event_is_of_the_allowlisted_schema() {
    let events = [
        FeedEvent::Epoch { epoch: 3 },
        FeedEvent::Revision(BoardRevision {
            epoch: 3,
            revision: 41,
        }),
        FeedEvent::Reset { after: 40 },
    ];
    for event in events {
        let data = event_json(&event);
        assert!(allowlisted(&data), "{data}");
        let framed = frame(&Told { id: 7, event });
        let lines: Vec<&str> = framed.trim_end().lines().collect();
        assert_eq!(lines.len(), 3, "id, event, data: {framed:?}");
        assert_eq!(lines[0], "id: 7");
        assert_eq!(
            lines[1],
            format!("event: {}", data["kind"].as_str().unwrap())
        );
        assert_eq!(lines[2], format!("data: {data}"));
    }
    let (status, body) = revisions_at(&routes(3).0, "after=0");
    assert_eq!(status, 200, "{body}");
    let listed = body["revisions"].as_array().unwrap();
    assert_eq!(listed.len(), 3);
    assert!(listed.iter().all(allowlisted), "{body}");
    let mut keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["epoch", "more", "revisions", "tip"]);
}

/// Red test (plan B10): a batch limit of 500 is enforced: 1 to 500 is
/// taken, none asks 500, and anything else is 400 `limit`.
#[test]
fn a_batch_is_at_most_500() {
    let (routes, feed) = routes(1_200);
    let (status, body) = revisions_at(&routes, "after=0&limit=500");
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["revisions"].as_array().unwrap().len(), 500);
    assert_eq!(
        (body["tip"].as_u64(), body["more"].as_bool()),
        (Some(1_200), Some(true))
    );
    let (_, body) = revisions_at(&routes, "after=1000");
    assert_eq!(body["revisions"].as_array().unwrap().len(), 200);
    assert_eq!(body["more"], false);
    assert_eq!(*feed.limits.lock().unwrap(), vec![500, 500]);
    for refused in [
        "after=0&limit=501",
        "after=0&limit=0",
        "after=0&limit=1000000",
    ] {
        let (status, body) = revisions_at(&routes, refused);
        assert_eq!(
            (status, body["code"].as_str()),
            (400, Some("limit")),
            "{refused}"
        );
    }
    assert_eq!(
        feed.limits.lock().unwrap().len(),
        2,
        "a refused batch reads nothing"
    );
}

#[test]
fn the_revisions_query_is_an_allowlist() {
    let (routes, feed) = routes(3);
    for refused in [
        "",
        "limit=5",
        "after=1&after=2",
        "after=-1",
        "after=1&title=x",
        "after=",
        "after=1x",
    ] {
        let (status, body) = revisions_at(&routes, refused);
        assert_eq!(
            (status, body["code"].as_str()),
            (400, Some("query")),
            "{refused:?}"
        );
    }
    assert!(feed.limits.lock().unwrap().is_empty());
}

#[test]
fn another_boards_feed_is_404_and_a_server_without_one_has_no_route() {
    let (routes, _) = routes(3);
    let reply = routes.answer(&request(
        "/v1/boards/fedcba9876543210/revisions",
        Some("after=0"),
        None,
    ));
    assert_eq!(reply.status, 404);
    let bare = routes_of(serving(&MemoryBoardStore::default()));
    let path = format!("/v1/boards/{BOARD}/revisions");
    assert_eq!(
        bare.answer(&request(&path, Some("after=0"), None)).status,
        404
    );
    assert!(!bare.streams("/v1/status"));
    assert!(bare.streams(&format!("/v1/boards/{BOARD}/events")));
}

/// The bytes a stream sends until it has nothing more.
fn streamed(stream: &mut Box<dyn ServiceStream>) -> String {
    let mut sent = String::new();
    loop {
        match stream.next() {
            StreamStep::Send { bytes, more } => {
                sent.push_str(std::str::from_utf8(&bytes).unwrap());
                if !more {
                    return sent;
                }
            }
            StreamStep::End(bytes) => {
                sent.push_str(std::str::from_utf8(&bytes).unwrap());
                return sent;
            }
        }
    }
}

#[test]
fn a_stream_opens_from_its_last_event_id_else_its_after_else_the_tip() {
    let (routes, _) = routes(4);
    let path = format!("/v1/boards/{BOARD}/events");
    let open = |query, last| {
        routes
            .stream(&request(&path, query, last), Arc::new(|| {}))
            .unwrap_or_else(|reply| panic!("{reply:?}"))
    };
    let resumed = streamed(&mut open(Some("after=1"), Some("2")));
    assert!(resumed.starts_with("retry: 1000\n\n"), "{resumed}");
    assert!(!resumed.contains("id: 2\nevent: revision"), "{resumed}");
    assert!(resumed.contains("id: 3\nevent: revision"), "{resumed}");
    assert!(resumed.contains("id: 4\nevent: revision"), "{resumed}");
    let after = streamed(&mut open(Some("after=3"), None));
    assert!(
        after.contains("id: 4\nevent: revision") && !after.contains("id: 3\nevent: revision"),
        "{after}"
    );
    let now = streamed(&mut open(None, None));
    assert!(
        now.ends_with("id: 4\nevent: epoch\ndata: {\"kind\":\"epoch\",\"epoch\":1}\n\n"),
        "{now}"
    );
    for (query, last, code) in [
        (None, Some("x"), "last_event_id"),
        (Some("after=1&limit=2"), None, "query"),
    ] {
        let refused = routes.stream(&request(&path, query, last), Arc::new(|| {}));
        let body: Value = serde_json::from_slice(&refused.err().unwrap().body).unwrap();
        assert_eq!(body["code"], code);
    }
}

/// Review (#2756 L3): a limit of 21 digits or more is out of range, 400
/// `limit` like any other, not a malformed query.
#[test]
fn a_limit_of_any_length_out_of_range_is_400_limit() {
    let (routes, feed) = routes(3);
    for refused in [
        "after=0&limit=999999999999999999999",
        "after=0&limit=00000000000000000000001000",
    ] {
        let (status, body) = revisions_at(&routes, refused);
        assert_eq!(
            (status, body["code"].as_str()),
            (400, Some("limit")),
            "{refused}"
        );
    }
    let (status, body) = revisions_at(&routes, "after=0&limit=00000000000000000000003");
    assert_eq!(status, 200, "leading zeros are still a number: {body}");
    assert_eq!(*feed.limits.lock().unwrap(), vec![3]);
}

/// Review (#2756 L2): over TCP a request the gate let through with no
/// principal reaches neither feed route: 401 `token`.
#[test]
fn both_feed_routes_over_tcp_without_a_principal_are_401() {
    let (routes, feed) = routes(3);
    let tcp = |path: String, query: Option<&str>| ServiceRequest {
        transport: Transport::Tcp,
        ..request(&path, query, None)
    };
    let events = tcp(format!("/v1/boards/{BOARD}/events"), None);
    let refused = routes
        .stream(&events, Arc::new(|| {}))
        .err()
        .expect("refused");
    let body: Value = serde_json::from_slice(&refused.body).unwrap();
    assert_eq!(
        (refused.status, body["code"].as_str()),
        (401, Some("token"))
    );
    let revisions = tcp(format!("/v1/boards/{BOARD}/revisions"), Some("after=0"));
    let reply = routes.answer(&revisions);
    let body: Value = serde_json::from_slice(&reply.body).unwrap();
    assert_eq!((reply.status, body["code"].as_str()), (401, Some("token")));
    assert!(feed.limits.lock().unwrap().is_empty(), "nothing was read");
}

/// Review (#2756 L4): `answer` asked for the events route itself (never
/// the listener, which streams it) says to read it as a stream.
#[test]
fn the_events_route_answered_whole_says_to_stream_it() {
    let (routes, _) = routes(3);
    let reply = routes.answer(&request(&format!("/v1/boards/{BOARD}/events"), None, None));
    let body: Value = serde_json::from_slice(&reply.body).unwrap();
    assert_eq!((reply.status, body["code"].as_str()), (400, Some("stream")));
}
