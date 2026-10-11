//! The board operations every front end serves (#2477): one typed request
//! ([`BoardRequest`]) run by [`BoardOperations`] over the board's use cases
//! and listings into one typed outcome. The front ends (the `quecto board`
//! CLI, the agent `board` tool) read and answer JSON in the interface
//! layer; here nothing is JSON.
mod run;

/// `BoardListings` over `QueryBoard`, for the front ends' tests.
#[cfg(test)]
#[path = "query_listings_tests.rs"]
pub(crate) mod query_listings;

pub use run::{BoardOperations, BoardOutcome, BoardRequest, ListFilter};
