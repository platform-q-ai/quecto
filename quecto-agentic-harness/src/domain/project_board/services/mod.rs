//! The board's rules: which status moves a task makes and who makes them,
//! how claims are renewed and taken over, which pull requests a task links,
//! how a lost run is ended and how a draft is edited. Every rule returns a new
//! [`Task`](super::entities::task::Task), so the schema holds after each.
pub mod claims;
pub mod drafts;
pub mod pull_requests;
pub mod runs;
pub mod transitions;

#[cfg(test)]
#[path = "fixtures_tests.rs"]
mod fixtures_tests;
