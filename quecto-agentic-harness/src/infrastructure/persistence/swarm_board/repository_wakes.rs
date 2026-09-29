//! `BoardWakes` over the SQLite store (#2276): red-phase stub.
use crate::application::swarm::dto::NotificationCursor;
use crate::application::swarm::ports::BoardWakes;
use crate::domain::swarm::{BoardError, NotificationEvent, NotificationState, RefusalKind};

use super::repository::SqliteBoard;

fn pending() -> BoardError {
    BoardError::new(RefusalKind::Internal, "wake frontiers are not served yet")
}

impl BoardWakes for SqliteBoard<'_> {
    fn notification_events(&self, _actor: &str) -> Result<Vec<NotificationEvent>, BoardError> {
        Err(pending())
    }
    fn advance_notifications(&self, _actor: &str) -> Result<NotificationCursor, BoardError> {
        Err(pending())
    }
    fn notification_state(&self) -> Result<NotificationState, BoardError> {
        Err(pending())
    }
    fn create_wake_cursors(&self) -> Result<(), BoardError> {
        Err(pending())
    }
    fn event_generation(&self) -> Result<i64, BoardError> {
        Err(pending())
    }
    fn wake_cursor(&self, _actor: &str) -> Result<Option<i64>, BoardError> {
        Err(pending())
    }
    fn wake_events(
        &self,
        _previous: i64,
        _generation: i64,
        _actor: &str,
    ) -> Result<Vec<NotificationEvent>, BoardError> {
        Err(pending())
    }
    fn set_wake_cursor(&self, _actor: &str, _generation: i64) -> Result<(), BoardError> {
        Err(pending())
    }
}
