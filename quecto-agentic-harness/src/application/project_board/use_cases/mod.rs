//! The project board's use cases (#2476).
mod accept_plan;
mod board_change;
pub mod board_view;
mod bootstrap_project_board;
mod change_board_task;
mod clear_set_aside;
mod complete_merged_task;
mod create_board_task;
mod description_cells;
mod discard_plan;
mod edit_plan;
mod end_run;
mod fold_back_run;
mod list_left_runs;
mod plan_text;
mod propose_plan;
mod query_board;
mod run_board_operation;
mod serve_board;
mod start_board_task;
mod task_pointers;
mod update_draft_task;

pub use accept_plan::AcceptPlan;
pub use board_change::MAX_WRITE_ATTEMPTS;
pub use bootstrap_project_board::BootstrapProjectBoard;
pub use change_board_task::ChangeBoardTask;
pub use clear_set_aside::ClearSetAside;
pub use complete_merged_task::CompleteMergedTask;
pub use create_board_task::CreateBoardTask;
pub use discard_plan::DiscardPlan;
pub use edit_plan::EditPlan;
pub use end_run::EndRun;
pub use fold_back_run::FoldBackRun;
pub use list_left_runs::ListLeftRuns;
pub use propose_plan::ProposePlan;
pub use query_board::QueryBoard;
pub use run_board_operation::RunBoardOperation;
pub use serve_board::ServeBoard;
pub use start_board_task::StartBoardTask;
pub use update_draft_task::UpdateDraftTask;

#[cfg(test)]
#[path = "fixtures_tests.rs"]
pub(in crate::application::project_board) mod fixtures_tests;
