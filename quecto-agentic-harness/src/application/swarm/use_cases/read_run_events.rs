//! `Workbench.events(after=0, limit=25)` (#2277): the audit history, a
//! page at a time by durable event id.
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;
use crate::application::swarm::board_operation::operation;
use crate::application::swarm::board_read_models::{page_bounds, reading};
use crate::application::swarm::dto::{EventPage, ReadRunEventsRequest};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{BoardError, RefusalKind};

/// The cursor must be an integer of at least 0 and the limit an integer
/// from 1 to 100, checked before the gate. Through the read-only gate, the
/// events after the cursor in id order: one more than the limit is read,
/// so the page knows whether more follow; the next cursor is the page's
/// last id, or the given one for an empty page.
pub struct ReadRunEvents {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ReadRunEvents {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// The page's bounds, an authorisation refusal, or the store's.
    pub fn execute(&self, request: ReadRunEventsRequest) -> Result<EventPage, BoardError> {
        let Some((after, limit)) = page_bounds(&request.after, &request.limit) else {
            return Err(BoardError::new(
                RefusalKind::Invalid,
                "event page requires nonnegative cursor and limit 1 through 100",
            ));
        };
        operation(
            &*self.repository,
            &*self.clock,
            &request.actor,
            reading(),
            |transaction, _| {
                let mut events = transaction.event_page(after, limit + 1)?;
                let limit = usize::try_from(limit).unwrap_or(usize::MAX);
                let has_more = events.len() > limit;
                events.truncate(limit);
                let cursor = match events.last() {
                    Some(last) => last.get("id").and_then(Value::as_i64),
                    None => i64::try_from(after).ok(),
                };
                let cursor = cursor.ok_or_else(|| {
                    BoardError::new(RefusalKind::Store, "an event id is not an integer")
                })?;
                Ok(EventPage {
                    events,
                    cursor,
                    has_more,
                })
            },
        )
    }
}

impl OverRepository for ReadRunEvents {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "read_run_events_tests.rs"]
mod tests;
