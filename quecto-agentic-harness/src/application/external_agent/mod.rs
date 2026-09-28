//! An external agent run as a member's "brain" (epic #2284). This slice
//! (#2285) holds the pure projection of its event stream onto the
//! sub-agent protocol's values; the session use case and its ports
//! follow in later slices.

pub mod projection;
