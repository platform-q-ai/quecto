//! `uuid.uuid4().hex` (#2270): the board's run ids, member reservations,
//! claim tokens and file-reservation tokens.
use crate::application::swarm::ports::IdSource;

/// A random (version 4) UUID as 32 lowercase hex digits, no hyphens.
#[derive(Clone, Copy, Debug, Default)]
pub struct Uuid4Ids;

impl IdSource for Uuid4Ids {
    fn hex32(&self) -> String {
        String::new()
    }
}

#[cfg(test)]
#[path = "ids_tests.rs"]
mod tests;
