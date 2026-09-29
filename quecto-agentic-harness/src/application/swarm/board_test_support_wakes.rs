//! Test support (compiled only under `cfg(test)`): the in-memory board's
//! wake frontiers (#2276). An event's id is its position in the log plus
//! one; `wake_cursors` is `None` until a claim creates it, so a test sees
//! whether (and when) it was created. The SQLite adapter's contract tests
//! pin the real statements.
use std::collections::BTreeSet;

use serde_json::Value;

use super::{MemoryTransaction, RecordedEvent};
use crate::application::swarm::dto::NotificationCursor;
use crate::application::swarm::ports::BoardWakes;
use crate::domain::swarm::{
    BoardError, NotificationEvent, NotificationState, TaskState, TaskSummary,
};

/// Event `index` (0-based) as the policy reads it, with its id.
fn numbered(index: usize, event: &RecordedEvent) -> (i64, NotificationEvent) {
    let id = i64::try_from(index + 1).unwrap();
    (
        id,
        NotificationEvent {
            action: event.action.clone(),
            detail: event.detail.clone(),
            actor: Some(event.actor.clone()),
        },
    )
}

fn cursor_of(cursors: &[(String, i64)], actor: &str) -> Option<i64> {
    cursors
        .iter()
        .find(|(owner, _)| owner == actor)
        .map(|(_, event)| *event)
}

fn replace(cursors: &mut Vec<(String, i64)>, actor: &str, event: i64) {
    cursors.retain(|(owner, _)| owner != actor);
    cursors.push((actor.to_owned(), event));
}

impl BoardWakes for MemoryTransaction<'_> {
    fn notification_events(&self, actor: &str) -> Result<Vec<NotificationEvent>, BoardError> {
        let state = self.state.borrow();
        let after = cursor_of(&state.notification_cursors, actor).unwrap_or(0);
        Ok(state
            .events
            .iter()
            .enumerate()
            .map(|(index, event)| numbered(index, event))
            .filter(|(id, event)| *id > after && event.actor.as_deref() == Some(actor))
            .map(|(_, event)| event)
            .collect())
    }

    fn advance_notifications(&self, actor: &str) -> Result<NotificationCursor, BoardError> {
        self.note(format!("advance_notifications {actor}"));
        let current = self.event_generation()?;
        let mut state = self.state.borrow_mut();
        let previous = cursor_of(&state.notification_cursors, actor);
        replace(&mut state.notification_cursors, actor, current);
        Ok(NotificationCursor { previous, current })
    }

    fn notification_state(&self) -> Result<NotificationState, BoardError> {
        let state = self.state.borrow();
        let tasks = state
            .tasks
            .iter()
            .filter_map(|row| {
                Some(TaskSummary {
                    id: row.get("id")?.as_i64()?,
                    status: TaskState::new(row.get("status")?.as_str()?),
                    dependencies: row
                        .get("dependencies")
                        .and_then(Value::as_array)
                        .map(|ids| ids.iter().filter_map(Value::as_i64).collect())
                        .unwrap_or_default(),
                    owner: row.get("owner").and_then(Value::as_str).map(str::to_owned),
                })
            })
            .collect();
        let unread: BTreeSet<i64> = state
            .messages
            .iter()
            .filter(|message| message.status == "accepted")
            .map(|message| message.id)
            .collect();
        Ok(NotificationState { tasks, unread })
    }

    fn create_wake_cursors(&self) -> Result<(), BoardError> {
        let mut state = self.state.borrow_mut();
        if state.wake_cursors.is_none() {
            state.wake_cursors = Some(Vec::new());
            drop(state);
            self.note("create_wake_cursors".to_owned());
        }
        Ok(())
    }

    fn event_generation(&self) -> Result<i64, BoardError> {
        Ok(i64::try_from(self.state.borrow().events.len()).unwrap())
    }

    fn wake_cursor(&self, actor: &str) -> Result<Option<i64>, BoardError> {
        let state = self.state.borrow();
        let cursors = state
            .wake_cursors
            .as_ref()
            .expect("wake_cursors is created before it is read");
        Ok(cursor_of(cursors, actor))
    }

    fn wake_events(
        &self,
        previous: i64,
        generation: i64,
        actor: &str,
    ) -> Result<Vec<NotificationEvent>, BoardError> {
        let state = self.state.borrow();
        Ok(state
            .events
            .iter()
            .enumerate()
            .map(|(index, event)| numbered(index, event))
            .filter(|(id, event)| {
                *id > previous && *id <= generation && event.actor.as_deref() != Some(actor)
            })
            .map(|(_, event)| event)
            .collect())
    }

    fn set_wake_cursor(&self, actor: &str, generation: i64) -> Result<(), BoardError> {
        self.note(format!("set_wake_cursor {actor} {generation}"));
        let mut state = self.state.borrow_mut();
        let cursors = state
            .wake_cursors
            .as_mut()
            .expect("wake_cursors is created before it is written");
        replace(cursors, actor, generation);
        Ok(())
    }
}
