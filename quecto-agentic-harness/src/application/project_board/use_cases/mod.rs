//! The project board's use cases (#2476).
mod bootstrap_project_board;
mod query_board;

pub use bootstrap_project_board::BootstrapProjectBoard;
pub use query_board::QueryBoard;

#[cfg(test)]
#[path = "fixtures_tests.rs"]
mod fixtures_tests;
