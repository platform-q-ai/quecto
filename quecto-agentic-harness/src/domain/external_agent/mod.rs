//! An external agent run as a member's "brain" (epic #2284): the typed
//! vocabulary of its event stream and the pure rules for reading it
//! (#2285). There is no I/O here.

pub mod stream;
pub mod turn;
