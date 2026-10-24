//! The board service's transport (epic #2705 B4): HTTP/1.1 over a Unix
//! socket, served by hyper behind allowlist limits, and the blocking
//! client the CLI reaches it with. The listener knows no board: it hands
//! each admitted request to the handler it was given.
pub mod client;
pub mod limits;
pub mod listener;

#[cfg(test)]
#[path = "test_server_tests.rs"]
mod test_server;
