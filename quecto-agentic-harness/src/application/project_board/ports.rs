use crate::domain::project_board::entities::task::Task;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardSnapshot { pub head: String, pub tasks: Vec<Task> }

#[derive(Debug, PartialEq, Eq)]
pub enum PublishError { Conflict, Missing, Failed(String) }

pub trait ProjectBoardRepository {
    fn bootstrap(&self) -> Result<bool, PublishError>;
    fn snapshot(&self) -> Result<BoardSnapshot, PublishError>;
    fn publish(&self, base: &str, tasks: &[Task], message: &str) -> Result<String, PublishError>;
}
