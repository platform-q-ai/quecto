//! The [`BoardTextCells`] port over the board's markdown codec (S4): a
//! line is held exactly when the codec's own check of a text cell passes,
//! the check every artifact's codec makes of the text cells it writes.
use crate::application::project_board::ports::BoardTextCells;
use crate::infrastructure::persistence::project_board::markdown::check_text_cell;

/// Text cells as the git store's markdown files hold them.
pub struct MarkdownTextCells;

impl BoardTextCells for MarkdownTextCells {
    fn holds(&self, line: &str) -> bool {
        check_text_cell(line).is_ok()
    }
}

#[cfg(test)]
#[path = "text_cells_tests.rs"]
mod tests;
