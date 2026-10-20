//! The coordination board's wake state.

use crate::application::swarm::dto::NotificationCursor;
use crate::domain::swarm::services::notification::{NotificationEvent, NotificationState};
use crate::domain::swarm::value_objects::board_error::BoardError;

/// The wake frontiers (#2276): each member's `notification_cursors` row
/// (the last event its own hints were sent for) and its `wake_cursors`
/// row (the last generation it accepted a wake for), with the events and
/// the board state the wake policy judges. The actor is the caller's own
/// member id; an event detail is loaded as Python's `json.loads` loads it.
pub trait BoardWakes {
    /// `Transaction.notification_events(actor)`: the events `actor`
    /// recorded after its notification cursor (the stored value bound as
    /// read, `0` without a row), in id order, each naming `actor`.
    fn notification_events(&self, actor: &str) -> Result<Vec<NotificationEvent>, BoardError>;
    /// `advance_notifications(actor)`: `actor`'s notification cursor
    /// becomes the latest event id (`0` for none), by `INSERT OR REPLACE`.
    fn advance_notifications(&self, actor: &str) -> Result<NotificationCursor, BoardError>;
    /// `Transaction.notification_state()`: every task's id, status,
    /// dependencies and owner, and the ids of the unread messages.
    fn notification_state(&self) -> Result<NotificationState, BoardError>;
    /// `CREATE TABLE IF NOT EXISTS wake_cursors`: the table is created
    /// lazily, at a wake claim's start, inside its transaction.
    fn create_wake_cursors(&self) -> Result<(), BoardError>;
    /// `SELECT coalesce(max(id),0) FROM events`: the board's generation.
    fn event_generation(&self) -> Result<i64, BoardError>;
    /// The generation `actor` last accepted a wake for; `None` without a
    /// row. The caller has created `wake_cursors` in this transaction.
    fn wake_cursor(&self, actor: &str) -> Result<Option<i64>, BoardError>;
    /// The events after `previous` up to `generation` that another member
    /// than `actor` recorded (`actor<>?`), in id order.
    fn wake_events(
        &self,
        previous: i64,
        generation: i64,
        actor: &str,
    ) -> Result<Vec<NotificationEvent>, BoardError>;
    /// `INSERT OR REPLACE INTO wake_cursors VALUES(?,?)`.
    fn set_wake_cursor(&self, actor: &str, generation: i64) -> Result<(), BoardError>;
}
