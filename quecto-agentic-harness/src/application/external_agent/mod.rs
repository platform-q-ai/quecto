//! An external agent run as a member's "brain" (epic #2284). This slice
//! (#2285) holds the capability's DTOs and the projection of its event
//! stream onto them: a capability-internal helper, not a use case (the
//! session use case of a later slice builds one per session).

pub mod dto;
pub mod projection;
