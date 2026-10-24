//! The board service's transport (epic #2705 B4, B5): HTTP/1.1 over a
//! Unix socket and TCP, served by hyper behind allowlist limits, and a
//! blocking client of the socket for the admin commands. The listener
//! knows no board and no token: it hands each admitted request, with its
//! transport, to the handler it was given.
pub mod limits;
pub mod listener;
pub mod local_client;

#[cfg(test)]
#[path = "test_server_tests.rs"]
pub(crate) mod test_server;
