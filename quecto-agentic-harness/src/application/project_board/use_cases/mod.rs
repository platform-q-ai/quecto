//! The project board's use cases (#2476).
mod accept_plan;
mod board_access;
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
mod finish_without_run;
mod fold_back_run;
mod list_left_runs;
mod plan_text;
mod propose_plan;
mod query_board;
mod run_board_operation;
mod serve_board;
mod serve_board_operation;
mod start_board_task;
mod task_pointers;
mod update_draft_task;

pub use accept_plan::AcceptPlan;
pub use board_access::{BoardAccess, Principal, Refused, Scope};
pub use board_change::MAX_WRITE_ATTEMPTS;
pub use bootstrap_project_board::BootstrapProjectBoard;
pub use change_board_task::ChangeBoardTask;
pub use clear_set_aside::ClearSetAside;
pub use complete_merged_task::{CompleteMergedTask, READ_BUDGET};
pub use create_board_task::CreateBoardTask;
pub use discard_plan::DiscardPlan;
pub use edit_plan::EditPlan;
pub use end_run::EndRun;
pub use finish_without_run::FinishWithoutRun;
pub use fold_back_run::FoldBackRun;
pub use list_left_runs::ListLeftRuns;
pub use propose_plan::ProposePlan;
pub use query_board::QueryBoard;
pub use run_board_operation::RunBoardOperation;
pub use serve_board::ServeBoard;
pub use serve_board_operation::{Present, ServeBoardOperation};
pub use start_board_task::StartBoardTask;
pub use update_draft_task::UpdateDraftTask;

#[cfg(test)]
#[path = "fixtures_tests.rs"]
pub(in crate::application::project_board) mod fixtures_tests;

mod export_board_history;
pub use export_board_history::ExportBoardHistory;

mod import_board_history;
pub use import_board_history::ImportBoardHistory;

mod backup_board;
pub use backup_board::BackupBoard;
