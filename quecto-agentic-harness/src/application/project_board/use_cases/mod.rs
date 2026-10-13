//! The project board's use cases (#2476).
mod board_change;
pub mod board_view;
mod bootstrap_project_board;
mod change_board_task;
mod clear_set_aside;
mod complete_merged_task;
mod create_board_task;
mod edit_plan;
mod plan_text;
mod propose_plan;
mod query_board;
mod run_board_operation;
mod task_pointers;
mod update_draft_task;

pub use board_change::MAX_WRITE_ATTEMPTS;
pub use bootstrap_project_board::BootstrapProjectBoard;
pub use change_board_task::ChangeBoardTask;
pub use clear_set_aside::ClearSetAside;
pub use complete_merged_task::CompleteMergedTask;
pub use create_board_task::CreateBoardTask;
pub use edit_plan::EditPlan;
pub use propose_plan::ProposePlan;
pub use query_board::QueryBoard;
pub use run_board_operation::RunBoardOperation;
pub use update_draft_task::UpdateDraftTask;

#[cfg(test)]
#[path = "fixtures_tests.rs"]
pub(in crate::application::project_board) mod fixtures_tests;
