//! Red stub (B4): the board server's graph.
pub use crate::interface::cli::board_server::{BoardServerEnvironment, BoardServerHandles};

/// # Errors
/// Not yet built.
pub fn build_board_server(
    environment: &BoardServerEnvironment,
) -> Result<BoardServerHandles, String> {
    Err(format!(
        "board-server: {} not yet built",
        environment.repo.display()
    ))
}

#[cfg(test)]
#[path = "board_server_tests.rs"]
mod tests;
