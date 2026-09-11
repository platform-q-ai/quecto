//! The single concrete graph for the find capability.
use std::path::PathBuf;
use std::sync::Arc;

use crate::application::agent_turn::use_cases::find::FindUseCase;
use crate::domain::tool::Tool;
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::find_fd::FdFindPaths;
use crate::interface::tools::find::FindTool;

pub fn build_find_tool(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>) -> Arc<dyn Tool> {
    Arc::new(FindTool::new(FindUseCase::new(Arc::new(FdFindPaths::new(
        workspace, sandbox,
    )))))
}

#[cfg(any(test, feature = "test-support"))]
pub fn build_find_tool_with_binary(
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
    binary: String,
) -> Arc<dyn Tool> {
    Arc::new(FindTool::new(FindUseCase::new(Arc::new(
        FdFindPaths::with_fd_binary(workspace, sandbox, binary),
    ))))
}
