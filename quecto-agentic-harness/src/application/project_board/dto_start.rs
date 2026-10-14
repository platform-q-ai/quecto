//! What a start takes (S4c): the task and the accepted plan version its
//! caller confirmed.
use crate::domain::project_board::value_objects::slug::Slug;

/// Task `task` started on its accepted plan, which the caller confirmed as
/// version `plan_version`: a task that names another now is refused, so
/// no plan the caller did not confirm is ever launched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskStart {
    pub task: Slug,
    pub plan_version: u32,
}
