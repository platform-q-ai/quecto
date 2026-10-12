//! The board's rules: which status moves a task makes and who makes them,
//! how claims are renewed and taken over, which pull requests a task links,
//! how a draft is edited, how a task's plan versions are proposed and
//! accepted, and how a run folds back or is ended as lost. Every rule
//! returns new artifacts, so each one's schema holds after it.
pub mod changes;
pub mod claims;
pub mod drafts;
pub mod plans;
pub mod pull_requests;
pub mod reviews;
pub mod runs;
pub mod transitions;

#[cfg(test)]
#[path = "fixtures_tests.rs"]
pub(in crate::domain::project_board) mod fixtures_tests;
