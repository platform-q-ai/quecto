//! Test support (compiled only under `cfg(test)`): the in-memory board's
//! read-model reads (#2277): the run as `dict(row)`, member statuses, the
//! grouped latest activity (each call journalled, so a test counts the
//! scans), event times and pages.
use serde_json::{Value, json};

use super::{BoardState, RecordedEvent, ids::sorted};
use crate::application::swarm::dto::{DictRow, LatestActivity, MemberRow};

/// `SELECT * FROM run` as `dict(row)`, the board's columns in order.
pub(super) fn run_row(state: &BoardState) -> Option<DictRow> {
    let run = state.run.as_ref()?;
    let text = |value: &Option<String>| value.clone().map_or(Value::Null, Value::String);
    let record = &run.record;
    Some(DictRow {
        columns: vec![
            ("id".to_owned(), json!(run.id)),
            ("goal".to_owned(), json!(run.contract.goal)),
            ("constraints".to_owned(), run.contract.constraints.clone()),
            ("criteria".to_owned(), run.contract.criteria.clone()),
            ("coordinator".to_owned(), text(&record.coordinator)),
            ("integrator".to_owned(), text(&record.coordinator)),
            ("member_limit".to_owned(), json!(record.member_limit)),
            ("deadline".to_owned(), json!(record.deadline)),
            (
                "status".to_owned(),
                record
                    .status
                    .as_ref()
                    .map_or(Value::Null, |status| json!(status.as_str())),
            ),
            ("outcome".to_owned(), text(&record.outcome)),
            ("outcome_reason".to_owned(), text(&record.outcome_reason)),
        ],
    })
}

/// `SELECT id,status FROM members WHERE id IN (…)`.
pub(super) fn member_statuses(
    members: &[MemberRow],
    ids: &[&str],
) -> Vec<(String, Option<String>)> {
    members
        .iter()
        .filter_map(|row| {
            let id = row.text("id")?;
            ids.contains(&id)
                .then(|| (id.to_owned(), row.text("status").map(str::to_owned)))
        })
        .collect()
}

/// The grouped `max(time)` of each of `actors`' events.
pub(super) fn latest_activity(events: &[RecordedEvent], actors: &[&str]) -> Vec<LatestActivity> {
    actors
        .iter()
        .filter_map(|actor| {
            let latest = events
                .iter()
                .filter(|event| event.actor == *actor)
                .map(|event| event.time)
                .reduce(f64::max)?;
            Some(LatestActivity {
                actor: (*actor).to_owned(),
                latest: json!(latest),
            })
        })
        .collect()
}

/// Event `id`'s time (ids count from 1 in write order).
pub(super) fn event_time(events: &[RecordedEvent], id: i64) -> Option<Value> {
    let index = usize::try_from(id).ok()?.checked_sub(1)?;
    events.get(index).map(|event| json!(event.time))
}

/// `SELECT * FROM events WHERE id>? ORDER BY id LIMIT ?`, each detail as
/// the board's sorted, compact text.
pub(super) fn event_page(events: &[RecordedEvent], after: u64, limit: i64) -> Vec<DictRow> {
    let skip = usize::try_from(after).unwrap_or(usize::MAX);
    let take = usize::try_from(limit).unwrap_or(0);
    events
        .iter()
        .enumerate()
        .skip(skip)
        .take(take)
        .map(|(index, event)| DictRow {
            columns: vec![
                ("id".to_owned(), json!(index + 1)),
                ("actor".to_owned(), json!(event.actor)),
                ("time".to_owned(), json!(event.time)),
                ("action".to_owned(), json!(event.action)),
                (
                    "detail".to_owned(),
                    json!(serde_json::to_string(&sorted(&event.detail)).unwrap()),
                ),
            ],
        })
        .collect()
}
