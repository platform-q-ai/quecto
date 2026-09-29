//! Stub (#2272 red phase).
use serde_json::Value;

use super::repository::SqliteBoard;
use crate::application::swarm::dto::{NewTask, TaskRow, TaskUpdate};
use crate::application::swarm::ports::{BoardFiles, BoardRequests, BoardTasks, RequestAction};
use crate::domain::swarm::BoardError;

fn pending<T>() -> Result<T, BoardError> {
    Err(BoardError::new("pending #2272"))
}

impl BoardTasks for SqliteBoard<'_> {
    fn task(&self, _id: &Value) -> Result<Option<TaskRow>, BoardError> {
        pending()
    }
    fn task_status(&self, _id: &Value) -> Result<Option<String>, BoardError> {
        pending()
    }
    fn all_task_dependencies(&self) -> Result<Vec<(i64, Value)>, BoardError> {
        pending()
    }
    fn task_count(&self) -> Result<i64, BoardError> {
        pending()
    }
    fn insert_task(&self, _task: &NewTask) -> Result<i64, BoardError> {
        pending()
    }
    fn set_task_dependencies(&self, _id: &Value, _dependencies: &Value) -> Result<(), BoardError> {
        pending()
    }
    fn update_task_claim(&self, _id: &Value, _owner: &str, _token: &str) -> Result<(), BoardError> {
        pending()
    }
    fn update_task_status(&self, _id: &Value, _update: &TaskUpdate) -> Result<(), BoardError> {
        pending()
    }
}

impl BoardRequests for SqliteBoard<'_> {
    fn retry(
        &self,
        _actor: &str,
        _request: &str,
        _payload: &Value,
        _action: &mut RequestAction<'_>,
    ) -> Result<Value, BoardError> {
        pending()
    }
}

impl BoardFiles for SqliteBoard<'_> {
    fn delete_claim_files(&self, _task: &Value, _claim: &Value) -> Result<(), BoardError> {
        pending()
    }
}
