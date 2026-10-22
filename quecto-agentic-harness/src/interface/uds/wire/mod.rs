//! Wire DTOs and protocol schema: the command and event shapes, and the
//! wire renderings shared by the controllers and presenters.
pub mod protocol;
pub(in crate::interface) mod uds_forward_response;
pub(in crate::interface) mod uds_freshness_json;
pub(in crate::interface) mod uds_visible_thinking_wire;
