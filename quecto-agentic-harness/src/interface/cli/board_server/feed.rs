//! The board's change feed over HTTP (epic #2705 B10):
//! `GET /v1/boards/{id}/events`, a `text/event-stream` from `Last-Event-ID`
//! (else `?after=`, else the tip), and `GET /v1/boards/{id}/revisions
//! ?after=<n>&limit=<1..500>`. Every event is one of an allowlisted schema
//! ([`event_json`]): numbers and a kind, never a task's text.
use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::application::project_board::ports::feed::{
    BatchLimit, FeedEvent, FeedWake, RevisionBatch, Told,
};
use crate::application::project_board::use_cases::{FollowBoard, FollowStep, Follower};
use crate::infrastructure::http::board_service::listener::{
    ServiceReply, ServiceRequest, ServiceStream, StreamStep, StreamWake,
};

/// What an idle stream sends: an SSE comment, which no reader dispatches.
const PING: &[u8] = b": ping\n\n";

/// An event's data: its kind and numbers, and nothing else.
pub fn event_json(event: &FeedEvent) -> Value {
    match event {
        FeedEvent::Epoch { epoch } => json!({"kind": "epoch", "epoch": epoch}),
        FeedEvent::Revision(revision) => json!({"kind": "revision",
            "epoch": revision.epoch, "revision": revision.revision}),
        FeedEvent::Reset { after } => json!({"kind": "reset", "after": after}),
    }
}

/// `told` as one SSE event: its id (the cursor to resume from), its kind
/// as the event's name, its data on one line.
fn frame(told: &Told) -> String {
    let data = event_json(&told.event);
    let kind = data["kind"].as_str().unwrap_or_default().to_owned();
    format!("id: {}\nevent: {kind}\ndata: {data}\n\n", told.id)
}

/// A follow as a stream: the first step made as it opens, so a store that
/// cannot be read is refused before any byte is sent.
struct FeedStream {
    follower: Follower,
    first: Option<StreamStep>,
}

impl FeedStream {
    fn followed(&mut self) -> StreamStep {
        match self.follower.step() {
            Ok(FollowStep::Events { told, more }) => StreamStep::Send {
                bytes: told.iter().map(frame).collect::<String>().into_bytes(),
                more,
            },
            Ok(FollowStep::Ended(told)) => {
                StreamStep::End(told.iter().map(frame).collect::<String>().into_bytes())
            }
            // The store failed mid-stream: it ends, and its reader resumes
            // from the last id it was sent.
            Err(_) => StreamStep::End(Vec::new()),
        }
    }
}

impl ServiceStream for FeedStream {
    fn next(&mut self) -> StreamStep {
        match self.first.take() {
            Some(first) => first,
            None => self.followed(),
        }
    }

    fn content_type(&self) -> &'static str {
        "text/event-stream"
    }

    fn keepalive(&self) -> &'static [u8] {
        PING
    }
}

/// The feed's stream for `request`: from its `Last-Event-ID`, else its
/// `?after=`, else the tip.
pub(super) fn open(
    feed: &FollowBoard,
    request: &ServiceRequest,
    wake: StreamWake,
) -> Result<Box<dyn ServiceStream>, ServiceReply> {
    let query = numbers(request.query.as_deref(), &["after"])?;
    let after =
        match request.last_event_id.as_deref() {
            Some(id) => Some(Told::parse_id(id).ok_or_else(|| {
                refused("last_event_id", "Last-Event-ID must be a revision number")
            })?),
            None => query.get("after").copied(),
        };
    let wake: FeedWake = wake;
    let mut stream = FeedStream {
        follower: feed.follow(after, wake),
        first: None,
    };
    let first = match stream.follower.step() {
        Ok(FollowStep::Events { told, more }) => StreamStep::Send {
            bytes: opening(&told),
            more,
        },
        Ok(FollowStep::Ended(told)) => StreamStep::End(opening(&told)),
        Err(error) => return Err(super::routes::failed(&error)),
    };
    stream.first = Some(first);
    Ok(Box::new(stream))
}

/// The stream's first bytes: the reconnection delay, then `told`.
fn opening(told: &[Told]) -> Vec<u8> {
    let events: String = told.iter().map(frame).collect();
    format!("retry: 1000\n\n{events}").into_bytes()
}

/// `GET /revisions?after=<n>[&limit=<1..500>]`: the revisions after `n`
/// as revision events, the tip, and whether more follow the batch.
pub(super) fn revisions(feed: &FollowBoard, request: &ServiceRequest) -> ServiceReply {
    let asked = numbers(request.query.as_deref(), &["after", "limit"]).and_then(|query| {
        let after = *query
            .get("after")
            .ok_or_else(|| refused("query", "after=<revision> is required"))?;
        let limit = match query.get("limit") {
            None => BatchLimit::MOST,
            Some(limit) => usize::try_from(*limit)
                .ok()
                .and_then(BatchLimit::of)
                .ok_or_else(|| refused("limit", "limit must be 1 to 500"))?,
        };
        Ok((after, limit))
    });
    let (after, limit) = match asked {
        Ok(asked) => asked,
        Err(reply) => return reply,
    };
    match feed.revisions(after, limit) {
        Ok(batch) => ServiceReply::json(200, &batch_json(&batch)),
        Err(error) => super::routes::failed(&error),
    }
}

fn batch_json(batch: &RevisionBatch) -> Value {
    let events: Vec<Value> = batch
        .revisions
        .iter()
        .map(|revision| event_json(&FeedEvent::Revision(*revision)))
        .collect();
    let last = batch.revisions.last().map(|last| last.revision);
    let more = last.is_some_and(|last| last < batch.tip.revision);
    json!({"revisions": events, "epoch": batch.tip.epoch, "tip": batch.tip.revision,
           "more": more})
}

/// The query's `key=<digits>` pairs, each key of `keys` at most once; any
/// other key, a repeat or a value not 1 to 64 digits is refused (400
/// `query`). A value past `u64` reads as `u64::MAX`: out of any range.
fn numbers<'k>(
    query: Option<&str>,
    keys: &[&'k str],
) -> Result<BTreeMap<&'k str, u64>, ServiceReply> {
    let mut found = BTreeMap::new();
    let pairs = query
        .unwrap_or_default()
        .split('&')
        .filter(|pair| !pair.is_empty());
    for pair in pairs {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let known = keys.iter().find(|known| **known == key);
        let (Some(key), Some(value)) = (known, digits(value)) else {
            let keys = keys.join(", ");
            let why = format!("the query takes only {keys}, each once, as a number");
            return Err(refused("query", &why));
        };
        if found.insert(*key, value).is_some() {
            return Err(refused("query", &format!("{key} is given twice")));
        }
    }
    Ok(found)
}

/// `text` as a query number: 1 to 64 ASCII digits, `u64::MAX` past it.
fn digits(text: &str) -> Option<u64> {
    let shaped = (1..=64).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit());
    shaped.then(|| text.parse().unwrap_or(u64::MAX))
}

fn refused(code: &str, why: &str) -> ServiceReply {
    ServiceReply::json(400, &json!({ "error": why, "code": code }))
}

#[cfg(test)]
#[path = "feed_tests.rs"]
mod tests;
