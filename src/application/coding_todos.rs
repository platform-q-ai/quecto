//! Per-job todo tracking for the coding coordinator.
//!
//! Workers propose todo updates through events. The coordinator validates
//! transitions and maintains the canonical todo list per job.

use std::collections::HashMap;

use crate::domain::coding_command::TodoItem;

/// Default maximum todo items per job when not configured.
const DEFAULT_MAX_ITEMS: usize = 200;

/// Maximum byte length for todo_id, title, owner, and depends_on entries.
const MAX_FIELD_LEN: usize = 512;

/// Errors from todo operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TodoError {
    /// Job not found.
    JobNotFound,
    /// Todo with this ID already exists.
    DuplicateId,
    /// Job has reached the max todo item limit.
    LimitReached,
    /// Todo with this ID not found.
    TodoNotFound,
    /// Invalid state transition.
    InvalidTransition,
    /// A field exceeds the maximum allowed length.
    InvalidField,
}

impl std::fmt::Display for TodoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::JobNotFound => write!(f, "job_not_found"),
            Self::DuplicateId => write!(f, "duplicate_id"),
            Self::LimitReached => write!(f, "limit_reached"),
            Self::TodoNotFound => write!(f, "todo_not_found"),
            Self::InvalidTransition => write!(f, "invalid_transition"),
            Self::InvalidField => write!(f, "invalid_field"),
        }
    }
}

/// Parameters for creating a new todo.
pub struct TodoCreateParams {
    pub todo_id: String,
    pub title: String,
    pub owner: Option<String>,
    pub depends_on: Vec<String>,
}

/// Parameters for completing a todo.
pub struct TodoCompleteParams<'a> {
    pub todo_id: &'a str,
    pub result: Option<String>,
    pub artifact_refs: Vec<String>,
}

/// Parameters for updating a todo's status.
pub struct TodoUpdateParams<'a> {
    pub todo_id: &'a str,
    pub new_status: &'a str,
    pub note: Option<String>,
}

/// Parameters for blocking a todo.
pub struct TodoBlockedParams<'a> {
    pub todo_id: &'a str,
    pub reason: String,
    pub needs: Option<String>,
}

/// Tracks todos per job. Owns all todo state and transition validation.
#[derive(Debug, Default)]
pub struct TodoTracker {
    /// Todos indexed by job_id -> list of TodoItem.
    todos_by_job: HashMap<String, Vec<TodoItem>>,
    /// Completion results by (job_id, todo_id).
    results: HashMap<(String, String), String>,
    /// Blocked reasons by (job_id, todo_id).
    blocked_reasons: HashMap<(String, String), String>,
    /// Blocked needs by (job_id, todo_id).
    blocked_needs: HashMap<(String, String), String>,
    /// Notes by (job_id, todo_id).
    notes: HashMap<(String, String), String>,
    /// Max items per job.
    max_items_per_job: usize,
}

impl TodoTracker {
    pub fn new() -> Self {
        Self {
            todos_by_job: HashMap::new(),
            results: HashMap::new(),
            blocked_reasons: HashMap::new(),
            blocked_needs: HashMap::new(),
            notes: HashMap::new(),
            max_items_per_job: DEFAULT_MAX_ITEMS,
        }
    }

    /// Maximum allowed value for per-job item limit.
    const MAX_LIMIT: usize = 10_000;

    pub fn set_max_items_per_job(&mut self, limit: usize) {
        self.max_items_per_job = limit.clamp(1, Self::MAX_LIMIT);
    }

    pub fn todos_for_job(&self, job_id: &str) -> &[TodoItem] {
        self.todos_by_job
            .get(job_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn todo_result(&self, job_id: &str, todo_id: &str) -> Option<&str> {
        self.results
            .get(&(job_id.to_string(), todo_id.to_string()))
            .map(|s| s.as_str())
    }

    pub fn blocked_reason(&self, job_id: &str, todo_id: &str) -> Option<&str> {
        self.blocked_reasons
            .get(&(job_id.to_string(), todo_id.to_string()))
            .map(|s| s.as_str())
    }

    pub fn blocked_needs(&self, job_id: &str, todo_id: &str) -> Option<&str> {
        self.blocked_needs
            .get(&(job_id.to_string(), todo_id.to_string()))
            .map(|s| s.as_str())
    }

    pub fn note(&self, job_id: &str, todo_id: &str) -> Option<&str> {
        self.notes
            .get(&(job_id.to_string(), todo_id.to_string()))
            .map(|s| s.as_str())
    }

    pub fn create_todo(&mut self, job_id: &str, params: TodoCreateParams) -> Result<(), TodoError> {
        validate_field_lengths(&params)?;
        let todos = self.todos_by_job.entry(job_id.to_string()).or_default();
        if todos.iter().any(|t| t.todo_id == params.todo_id) {
            return Err(TodoError::DuplicateId);
        }
        if todos.len() >= self.max_items_per_job {
            return Err(TodoError::LimitReached);
        }
        todos.push(TodoItem {
            todo_id: params.todo_id,
            title: params.title,
            status: "pending".to_string(),
            owner: params.owner,
            depends_on: params.depends_on,
            artifact_refs: vec![],
        });
        Ok(())
    }

    pub fn update_status(
        &mut self,
        job_id: &str,
        params: TodoUpdateParams<'_>,
    ) -> Result<(), TodoError> {
        let todos = self
            .todos_by_job
            .get_mut(job_id)
            .ok_or(TodoError::JobNotFound)?;
        let todo = todos
            .iter_mut()
            .find(|t| t.todo_id == params.todo_id)
            .ok_or(TodoError::TodoNotFound)?;
        if !can_transition(&todo.status, params.new_status) {
            return Err(TodoError::InvalidTransition);
        }
        todo.status = params.new_status.to_string();
        if let Some(n) = params.note {
            self.notes
                .insert((job_id.to_string(), params.todo_id.to_string()), n);
        }
        Ok(())
    }

    pub fn complete_todo(
        &mut self,
        job_id: &str,
        params: TodoCompleteParams<'_>,
    ) -> Result<(), TodoError> {
        let todos = self
            .todos_by_job
            .get_mut(job_id)
            .ok_or(TodoError::JobNotFound)?;
        let todo = todos
            .iter_mut()
            .find(|t| t.todo_id == params.todo_id)
            .ok_or(TodoError::TodoNotFound)?;
        if !can_transition(&todo.status, "completed") {
            return Err(TodoError::InvalidTransition);
        }
        todo.status = "completed".to_string();
        todo.artifact_refs = params.artifact_refs;
        if let Some(res) = params.result {
            self.results
                .insert((job_id.to_string(), params.todo_id.to_string()), res);
        }
        Ok(())
    }

    pub fn block_todo(
        &mut self,
        job_id: &str,
        params: TodoBlockedParams<'_>,
    ) -> Result<(), TodoError> {
        let todos = self
            .todos_by_job
            .get_mut(job_id)
            .ok_or(TodoError::JobNotFound)?;
        let todo = todos
            .iter_mut()
            .find(|t| t.todo_id == params.todo_id)
            .ok_or(TodoError::TodoNotFound)?;
        if !can_transition(&todo.status, "blocked") {
            return Err(TodoError::InvalidTransition);
        }
        todo.status = "blocked".to_string();
        self.blocked_reasons.insert(
            (job_id.to_string(), params.todo_id.to_string()),
            params.reason,
        );
        if let Some(needs) = params.needs {
            self.blocked_needs
                .insert((job_id.to_string(), params.todo_id.to_string()), needs);
        }
        Ok(())
    }

    /// Cancel all non-terminal todos for a job.
    pub fn cancel_all(&mut self, job_id: &str) {
        if let Some(todos) = self.todos_by_job.get_mut(job_id) {
            for todo in todos.iter_mut() {
                if matches!(todo.status.as_str(), "pending" | "in_progress" | "blocked") {
                    todo.status = "canceled".to_string();
                }
            }
        }
    }

    /// Remove all state for a job (called during cleanup).
    pub fn remove_job(&mut self, job_id: &str) {
        self.todos_by_job.remove(job_id);
        self.results.retain(|(jid, _), _| jid != job_id);
        self.blocked_reasons.retain(|(jid, _), _| jid != job_id);
        self.blocked_needs.retain(|(jid, _), _| jid != job_id);
        self.notes.retain(|(jid, _), _| jid != job_id);
    }
}

fn validate_field_lengths(params: &TodoCreateParams) -> Result<(), TodoError> {
    if params.todo_id.len() > MAX_FIELD_LEN
        || params.title.len() > MAX_FIELD_LEN
        || params
            .owner
            .as_ref()
            .is_some_and(|o| o.len() > MAX_FIELD_LEN)
        || params.depends_on.iter().any(|d| d.len() > MAX_FIELD_LEN)
    {
        return Err(TodoError::InvalidField);
    }
    Ok(())
}

fn can_transition(from: &str, to: &str) -> bool {
    if from == to {
        return true;
    }
    match from {
        "pending" => matches!(to, "in_progress" | "blocked" | "canceled"),
        "in_progress" => matches!(to, "blocked" | "completed" | "failed" | "canceled"),
        "blocked" => matches!(to, "in_progress" | "failed" | "canceled"),
        "completed" | "failed" | "canceled" => false,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker() -> TodoTracker {
        TodoTracker::new()
    }

    fn create_default(t: &mut TodoTracker, job_id: &str, todo_id: &str) {
        t.create_todo(
            job_id,
            TodoCreateParams {
                todo_id: todo_id.to_string(),
                title: "test".to_string(),
                owner: None,
                depends_on: vec![],
            },
        )
        .unwrap();
    }

    fn upd<'a>(todo_id: &'a str, status: &'a str, note: Option<String>) -> TodoUpdateParams<'a> {
        TodoUpdateParams {
            todo_id,
            new_status: status,
            note,
        }
    }

    #[test]
    fn test_create_todo() {
        let mut t = tracker();
        create_default(&mut t, "j1", "t1");
        let todos = t.todos_for_job("j1");
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].todo_id, "t1");
        assert_eq!(todos[0].status, "pending");
    }

    #[test]
    fn test_duplicate_rejected() {
        let mut t = tracker();
        create_default(&mut t, "j1", "t1");
        let err = t
            .create_todo(
                "j1",
                TodoCreateParams {
                    todo_id: "t1".to_string(),
                    title: "dup".to_string(),
                    owner: None,
                    depends_on: vec![],
                },
            )
            .unwrap_err();
        assert_eq!(err, TodoError::DuplicateId);
    }

    #[test]
    fn test_limit_reached() {
        let mut t = tracker();
        t.set_max_items_per_job(2);
        create_default(&mut t, "j1", "t1");
        create_default(&mut t, "j1", "t2");
        let err = t
            .create_todo(
                "j1",
                TodoCreateParams {
                    todo_id: "t3".to_string(),
                    title: "over".to_string(),
                    owner: None,
                    depends_on: vec![],
                },
            )
            .unwrap_err();
        assert_eq!(err, TodoError::LimitReached);
    }

    #[test]
    fn test_update_status_pending_to_in_progress() {
        let mut t = tracker();
        create_default(&mut t, "j1", "t1");
        t.update_status("j1", upd("t1", "in_progress", None))
            .unwrap();
        assert_eq!(t.todos_for_job("j1")[0].status, "in_progress");
    }

    #[test]
    fn test_complete_with_result() {
        let mut t = tracker();
        create_default(&mut t, "j1", "t1");
        t.update_status("j1", upd("t1", "in_progress", None))
            .unwrap();
        t.complete_todo(
            "j1",
            TodoCompleteParams {
                todo_id: "t1",
                result: Some("12 tests".to_string()),
                artifact_refs: vec![],
            },
        )
        .unwrap();
        assert_eq!(t.todos_for_job("j1")[0].status, "completed");
        assert_eq!(t.todo_result("j1", "t1"), Some("12 tests"));
    }

    #[test]
    fn test_complete_with_artifacts() {
        let mut t = tracker();
        create_default(&mut t, "j1", "t1");
        t.update_status("j1", upd("t1", "in_progress", None))
            .unwrap();
        t.complete_todo(
            "j1",
            TodoCompleteParams {
                todo_id: "t1",
                result: None,
                artifact_refs: vec!["test.log".to_string()],
            },
        )
        .unwrap();
        assert_eq!(t.todos_for_job("j1")[0].artifact_refs, vec!["test.log"]);
    }

    #[test]
    fn test_block_with_reason() {
        let mut t = tracker();
        create_default(&mut t, "j1", "t1");
        t.update_status("j1", upd("t1", "in_progress", None))
            .unwrap();
        t.block_todo(
            "j1",
            TodoBlockedParams {
                todo_id: "t1",
                reason: "failing test".to_string(),
                needs: None,
            },
        )
        .unwrap();
        assert_eq!(t.todos_for_job("j1")[0].status, "blocked");
        assert_eq!(t.blocked_reason("j1", "t1"), Some("failing test"));
    }

    #[test]
    fn test_block_with_needs() {
        let mut t = tracker();
        create_default(&mut t, "j1", "t1");
        t.update_status("j1", upd("t1", "in_progress", None))
            .unwrap();
        t.block_todo(
            "j1",
            TodoBlockedParams {
                todo_id: "t1",
                reason: "conflict".to_string(),
                needs: Some("main-agent decision".to_string()),
            },
        )
        .unwrap();
        assert_eq!(t.blocked_needs("j1", "t1"), Some("main-agent decision"));
    }

    #[test]
    fn test_invalid_transition_rejected() {
        let mut t = tracker();
        create_default(&mut t, "j1", "t1");
        // pending -> completed is invalid (must go through in_progress)
        let err = t
            .complete_todo(
                "j1",
                TodoCompleteParams {
                    todo_id: "t1",
                    result: Some("done".to_string()),
                    artifact_refs: vec![],
                },
            )
            .unwrap_err();
        assert_eq!(err, TodoError::InvalidTransition);
    }

    #[test]
    fn test_terminal_state_rejects_update() {
        let mut t = tracker();
        create_default(&mut t, "j1", "t1");
        t.update_status("j1", upd("t1", "in_progress", None))
            .unwrap();
        t.complete_todo(
            "j1",
            TodoCompleteParams {
                todo_id: "t1",
                result: None,
                artifact_refs: vec![],
            },
        )
        .unwrap();
        let err = t
            .update_status("j1", upd("t1", "in_progress", None))
            .unwrap_err();
        assert_eq!(err, TodoError::InvalidTransition);
    }

    #[test]
    fn test_cancel_all() {
        let mut t = tracker();
        create_default(&mut t, "j1", "t1");
        create_default(&mut t, "j1", "t2");
        t.update_status("j1", upd("t1", "in_progress", None))
            .unwrap();
        t.cancel_all("j1");
        assert!(t.todos_for_job("j1").iter().all(|t| t.status == "canceled"));
    }

    #[test]
    fn test_note() {
        let mut t = tracker();
        create_default(&mut t, "j1", "t1");
        t.update_status("j1", upd("t1", "in_progress", Some("8 of 12".to_string())))
            .unwrap();
        assert_eq!(t.note("j1", "t1"), Some("8 of 12"));
    }

    #[test]
    fn test_create_with_owner() {
        let mut t = tracker();
        t.create_todo(
            "j1",
            TodoCreateParams {
                todo_id: "t1".to_string(),
                title: "audit".to_string(),
                owner: Some("reviewer".to_string()),
                depends_on: vec![],
            },
        )
        .unwrap();
        assert_eq!(t.todos_for_job("j1")[0].owner.as_deref(), Some("reviewer"));
    }

    #[test]
    fn test_create_with_depends_on() {
        let mut t = tracker();
        create_default(&mut t, "j1", "t1");
        t.create_todo(
            "j1",
            TodoCreateParams {
                todo_id: "t2".to_string(),
                title: "run tests".to_string(),
                owner: None,
                depends_on: vec!["t1".to_string()],
            },
        )
        .unwrap();
        assert_eq!(t.todos_for_job("j1")[1].depends_on, vec!["t1"]);
    }

    #[test]
    fn test_remove_job_cleans_all_state() {
        let mut t = tracker();
        create_default(&mut t, "j1", "t1");
        t.update_status("j1", upd("t1", "in_progress", Some("note".to_string())))
            .unwrap();
        t.remove_job("j1");
        assert!(t.todos_for_job("j1").is_empty());
        assert!(t.note("j1", "t1").is_none());
    }

    #[test]
    fn test_blocked_to_in_progress() {
        let mut t = tracker();
        create_default(&mut t, "j1", "t1");
        t.update_status("j1", upd("t1", "in_progress", None))
            .unwrap();
        t.block_todo(
            "j1",
            TodoBlockedParams {
                todo_id: "t1",
                reason: "wait".to_string(),
                needs: None,
            },
        )
        .unwrap();
        t.update_status("j1", upd("t1", "in_progress", None))
            .unwrap();
        assert_eq!(t.todos_for_job("j1")[0].status, "in_progress");
    }

    #[test]
    fn test_blocked_to_failed() {
        let mut t = tracker();
        create_default(&mut t, "j1", "t1");
        t.update_status("j1", upd("t1", "in_progress", None))
            .unwrap();
        t.block_todo(
            "j1",
            TodoBlockedParams {
                todo_id: "t1",
                reason: "wait".to_string(),
                needs: None,
            },
        )
        .unwrap();
        t.update_status("j1", upd("t1", "failed", None)).unwrap();
        assert_eq!(t.todos_for_job("j1")[0].status, "failed");
    }

    #[test]
    fn test_pending_to_blocked() {
        let mut t = tracker();
        create_default(&mut t, "j1", "t1");
        t.block_todo(
            "j1",
            TodoBlockedParams {
                todo_id: "t1",
                reason: "upstream dep".to_string(),
                needs: None,
            },
        )
        .unwrap();
        assert_eq!(t.todos_for_job("j1")[0].status, "blocked");
    }

    #[test]
    fn test_failed_rejects_update() {
        let mut t = tracker();
        create_default(&mut t, "j1", "t1");
        t.update_status("j1", upd("t1", "in_progress", None))
            .unwrap();
        t.update_status("j1", upd("t1", "failed", None)).unwrap();
        let err = t
            .update_status("j1", upd("t1", "in_progress", None))
            .unwrap_err();
        assert_eq!(err, TodoError::InvalidTransition);
    }

    #[test]
    fn test_empty_job_returns_empty_slice() {
        let t = tracker();
        assert!(t.todos_for_job("nonexistent").is_empty());
    }

    #[test]
    fn test_oversized_todo_id_rejected() {
        let mut t = tracker();
        let err = t
            .create_todo(
                "j1",
                TodoCreateParams {
                    todo_id: "x".repeat(513),
                    title: "ok".to_string(),
                    owner: None,
                    depends_on: vec![],
                },
            )
            .unwrap_err();
        assert_eq!(err, TodoError::InvalidField);
    }

    #[test]
    fn test_set_max_items_clamped() {
        let mut t = tracker();
        t.set_max_items_per_job(0);
        // Should clamp to 1, so creating one todo succeeds
        create_default(&mut t, "j1", "t1");
        assert_eq!(t.todos_for_job("j1").len(), 1);
    }
}
