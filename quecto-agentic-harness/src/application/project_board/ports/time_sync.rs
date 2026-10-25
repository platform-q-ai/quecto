//! Whether the host's clock is synchronised (epic #2705): the board
//! server's clock gate. A server whose clock is not kept in step with an
//! outside time takes no change that reads the clock.

/// What the host says of its clock. Only [`Self::Synced`] lets the gate
/// through: anything the host does not affirm as synchronised is
/// [`Self::Unsynced`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClockSync {
    /// The host affirms its clock is synchronised to an outside time.
    Synced,
    /// Anything else, and why (said to the caller).
    Unsynced(String),
}

/// Port: the host's word on its clock. Cheap and non-blocking (one system
/// call), so it is asked on every reading of the clock.
pub trait TimeSync: Send + Sync {
    /// Whether the host's clock is synchronised now.
    fn sync(&self) -> ClockSync;
}
