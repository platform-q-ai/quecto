//! Wire DTOs and protocol schema: the command and event shapes, and the
//! wire renderings shared by the controllers and presenters.
pub mod parent_control_wire;
pub mod protocol;
pub mod subagent_teardown_wire;
pub(in crate::interface) mod uds_forward_response;
pub(in crate::interface) mod uds_freshness_json;
pub(in crate::interface) mod uds_visible_thinking_wire;
