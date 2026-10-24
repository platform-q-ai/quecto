//! The board service's transport (epic #2705 B4, B5): HTTP/1.1 over a Unix
//! socket and TCP, served by hyper behind allowlist limits. The listener knows no
//! board: it hands each admitted request to the handler it was given.
pub mod limits;
pub mod listener;
pub mod local_client;
mod sources;

#[cfg(test)]
#[path = "test_server_tests.rs"]
pub(crate) mod test_server;
