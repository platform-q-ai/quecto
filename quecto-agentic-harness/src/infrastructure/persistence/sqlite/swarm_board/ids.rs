//! `uuid.uuid4().hex` (#2270): the board's run ids, member reservations,
//! claim tokens and file-reservation tokens.
use crate::application::swarm::ports::IdSource;

/// A random (version 4) UUID as 32 lowercase hex digits, no hyphens.
#[derive(Clone, Copy, Debug, Default)]
pub struct Uuid4Ids;

impl IdSource for Uuid4Ids {
    fn hex32(&self) -> String {
        let id = uuid::Uuid::new_v4().simple().to_string();
        debug_assert!(
            id.len() == 32
                && id
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')),
            "uuid4().hex is 32 lowercase hex digits: {id}"
        );
        id
    }
}

#[cfg(test)]
#[path = "ids_tests.rs"]
mod tests;
