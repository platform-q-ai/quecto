//! STUB (#2273 red phase): `BoardUsage` over the SQLite store.
use super::repository::SqliteBoard;
use crate::application::swarm::dto::UsageReport;
use crate::application::swarm::ports::BoardUsage;
use crate::domain::swarm::BoardError;

impl BoardUsage for SqliteBoard<'_> {
    fn usage_report(&self) -> Result<UsageReport, BoardError> {
        Err(BoardError::new("pending #2273"))
    }
}
