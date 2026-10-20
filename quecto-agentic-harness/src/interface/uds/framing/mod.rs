//! Bounded transport framing: the socket, the per-connection wire mode,
//! the bounded reader, the single- and multi-client loops, and shutdown and
//! owner-exit delivery at the connection boundary.
pub(crate) mod uds_multi;
mod uds_multi_accept;
mod uds_owner_exit;
pub(in crate::interface) mod uds_reader;
pub(in crate::interface) mod uds_shutdown;
pub(in crate::interface) mod uds_single_client;
pub(in crate::interface) mod uds_socket;
pub mod uds_wire;
