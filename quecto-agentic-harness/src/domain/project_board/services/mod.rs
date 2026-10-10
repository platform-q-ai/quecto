//! The board's rules: which status moves a task makes and who makes them,
//! and how claims are renewed and taken over. Every rule returns a new
//! [`Task`](super::entities::task::Task), so the schema holds after each.
pub mod claims;
pub mod transitions;

#[cfg(test)]
#[path = "fixtures_tests.rs"]
mod fixtures_tests;
