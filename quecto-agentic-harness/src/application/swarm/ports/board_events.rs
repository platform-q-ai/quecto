//! The coordination board's `events` log.

use crate::application::swarm::dto::{DictRow, LatestActivity, ScopeObservation};
use crate::domain::swarm::value_objects::board_error::BoardError;
use serde_json::Value;

/// The `events` log.
pub trait BoardEvents {
    /// `Store.event`: one row by `actor` at `time` with the encoded detail.
    fn event(&self, actor: &str, time: f64, action: &str, detail: &Value)
    -> Result<(), BoardError>;
    /// The id of the latest `paused` or `resumed` event, `0` for none.
    fn control_generation(&self) -> Result<i64, BoardError>;
    /// `SELECT actor, time, detail FROM events WHERE
    /// action='scope_observed' ORDER BY id` (#1961, #2277): every loss
    /// observation, in id order.
    fn scope_observations(&self) -> Result<Vec<ScopeObservation>, BoardError>;
    /// `SELECT actor, max(time) latest FROM events WHERE actor IN (…) GROUP
    /// BY actor` (#1969, #2277): the owner liveness's one grouped scan, the
    /// latest time of each of `actors` that has an event.
    fn latest_activity(&self, actors: &[&str]) -> Result<Vec<LatestActivity>, BoardError>;
    /// `SELECT time FROM events WHERE id=?` (#2277): the event's time as
    /// stored, `None` when no event has that id.
    fn event_time(&self, id: i64) -> Result<Option<Value>, BoardError>;
    /// `SELECT * FROM events WHERE id>? ORDER BY id LIMIT ?` (#2277), each
    /// row as `dict(row)` with its detail the stored text. An `after`
    /// beyond SQLite's integers is refused as the store refuses an integer
    /// it cannot bind.
    fn event_page(&self, after: u64, limit: i64) -> Result<Vec<DictRow>, BoardError>;
    /// `SELECT time FROM events WHERE action='created' ORDER BY id DESC
    /// LIMIT 1` (#2313 review M1): when the run was created, `None` when no
    /// `created` event holds a number.
    fn created_at(&self) -> Result<Option<f64>, BoardError>;
}
