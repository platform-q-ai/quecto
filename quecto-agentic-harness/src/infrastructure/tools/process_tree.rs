//! Process topology vocabulary of a registered child.
//!
//! Since #1938 no code path in this module signals a pid: the only
//! signaller of a directly launched child is the owned-child supervisor
//! (#1935), through its retained handle. The topology is kept so the
//! supervisor knows whether a launch owns its own process group.

/// How this harness owns a registered child process for cleanup purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProcessOwner {
    /// Legacy/default: only the immediate pid is known to be owned.
    #[default]
    DirectPid,
    /// Unix local launch with `pgid == pid`; the supervisor terminates the
    /// whole process group through its handle.
    LocalProcessGroup,
}
