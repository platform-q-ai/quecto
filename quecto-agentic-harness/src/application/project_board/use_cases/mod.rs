//! The project board's use cases (#2476).
mod board_change;
mod bootstrap_project_board;
mod change_board_task;
mod complete_merged_task;
mod create_board_task;
mod query_board;
mod references;
mod update_draft_task;

pub use board_change::MAX_WRITE_ATTEMPTS;
pub use bootstrap_project_board::BootstrapProjectBoard;
pub use change_board_task::ChangeBoardTask;
pub use complete_merged_task::CompleteMergedTask;
pub use create_board_task::CreateBoardTask;
pub use query_board::QueryBoard;
pub use update_draft_task::UpdateDraftTask;

#[cfg(test)]
#[path = "fixtures_tests.rs"]
mod fixtures_tests;
