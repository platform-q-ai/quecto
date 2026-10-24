//! The board service's transport (epic #2705 B4): HTTP/1.1 over a Unix
//! socket, served by hyper behind allowlist limits. The listener knows no
//! board: it hands each admitted request to the handler it was given.
pub mod limits;
pub mod listener;

#[cfg(test)]
#[path = "test_server_tests.rs"]
pub(crate) mod test_server;
