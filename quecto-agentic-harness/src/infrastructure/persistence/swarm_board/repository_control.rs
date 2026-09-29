//! The control reads over the SQLite store (#2273), by Python's SQL
//! (`swarm_repository.py`): the latest pause record and the shared
//! `lost_members` scan, which `BoardRuns` and `BoardMembers` answer with.
//!
//! An event detail is loaded as Python's `json.loads` loads it; one that
//! is not JSON text is refused as a store failure where Python raises
//! (the `outside_edited_control_records` divergence), and a detail that is
//! not an object, or whose `member` is not text (a list or an object,
//! which Python cannot hash and raises on), names no member.
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;

use super::repository::{failed, fetched, text};
use super::repository_tasks::loaded;
use crate::domain::swarm::BoardError;

/// `Transaction.pause_started`'s read: the latest `paused` event's
/// `started`, NULL when its detail has none; `None` without such an event.
pub(super) fn pause_started(connection: &Connection) -> Result<Option<Value>, BoardError> {
    connection
        .query_row(
            "SELECT detail FROM events WHERE action='paused' ORDER BY id DESC LIMIT 1",
            [],
            |row| {
                fetched(row)?;
                let detail = loaded(row, 0)?;
                Ok(detail.get("started").cloned().unwrap_or(Value::Null))
            },
        )
        .optional()
        .map_err(failed)
}

/// The latest `scope_unknown` and `activated` event ids of one member.
#[derive(Clone, Copy, Default)]
struct Latest {
    scope_unknown: i64,
    activated: i64,
}

/// `swarm_repository.lost_members(connection, members)`: those of
/// `members` whose latest `scope_unknown` event is newer than their latest
/// `activated` one, in the order given.
pub(super) fn lost_members(
    connection: &Connection,
    members: &[&str],
) -> Result<Vec<String>, BoardError> {
    let mut latest = vec![None::<Latest>; members.len()];
    let mut statement = connection
        .prepare(
            "SELECT id, action, detail FROM events WHERE action IN ('scope_unknown','activated') ORDER BY id",
        )
        .map_err(failed)?;
    let rows = statement
        .query_map([], |row| {
            fetched(row)?;
            let detail = loaded(row, 2)?;
            Ok((row.get::<_, i64>("id")?, text(row, "action")?, detail))
        })
        .map_err(failed)?;
    for row in rows {
        let (id, action, detail) = row.map_err(failed)?;
        let named = detail.get("member").and_then(Value::as_str);
        let Some(index) = named.and_then(|named| members.iter().position(|m| *m == named)) else {
            continue;
        };
        let seen = latest[index].get_or_insert_with(Latest::default);
        match action.as_deref() {
            Some("scope_unknown") => seen.scope_unknown = id,
            Some("activated") => seen.activated = id,
            _ => {
                return Err(BoardError::new(
                    "the loss scan read an event it did not select",
                ));
            }
        }
    }
    Ok(members
        .iter()
        .zip(latest)
        .filter(|(_, seen)| seen.is_some_and(|seen| seen.scope_unknown > seen.activated))
        .map(|(member, _)| (*member).to_owned())
        .collect())
}
