//! STUB (#2273 red phase): the control reads over the SQLite store.
use rusqlite::Connection;
use serde_json::Value;

use crate::domain::swarm::BoardError;

pub(super) fn pause_started(_connection: &Connection) -> Result<Option<Value>, BoardError> {
    Err(BoardError::new("pending #2273"))
}

pub(super) fn lost_members(
    _connection: &Connection,
    _members: &[&str],
) -> Result<Vec<String>, BoardError> {
    Err(BoardError::new("pending #2273"))
}
