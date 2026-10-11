//! Creates a project's empty board, once.
use std::sync::Arc;

use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{Bootstrapped, ProjectBoardStore};

pub struct BootstrapProjectBoard {
    store: Arc<dyn ProjectBoardStore>,
}

impl BootstrapProjectBoard {
    pub fn new(store: Arc<dyn ProjectBoardStore>) -> Self {
        Self { store }
    }

    /// # Errors
    /// The store's clash or failure.
    pub fn execute(&self) -> Result<Bootstrapped, ProjectBoardError> {
        let _ = &self.store;
        Err(ProjectBoardError::Refused("not yet".into()))
    }
}

#[cfg(test)]
#[path = "bootstrap_project_board_tests.rs"]
mod tests;
