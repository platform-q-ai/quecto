//! The owner's exit announcement at the connection boundary (#2070). Read on
//! the reader task, which a running turn never blocks: the owning TUI sends
//! its exit persist and signals within seconds, so the announcement must not
//! wait behind the dispatch queue. The same connection's close withdraws it.
use crate::application::subagents::ports::OwnerExitAnnouncement;
use crate::domain::session::SubagentRestoreReason;

/// Raise the announcement when `line` is the owning TUI's exit persist. The
/// line is still forwarded to the dispatch loop, which performs the save.
pub(super) fn announce_if_exit_persist(
    owner_exit: &dyn OwnerExitAnnouncement,
    client_id: u64,
    line: &str,
) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return false;
    };
    let is_exit_persist = value["type"].as_str() == Some("persist_session")
        && value["restoreReason"].as_str()
            == Some(SubagentRestoreReason::ORDINARY_TUI_EXIT_STOPPED_WIRE);
    if is_exit_persist {
        owner_exit.announce(client_id);
    }
    is_exit_persist
}

#[cfg(test)]
#[path = "uds_owner_exit_tests.rs"]
mod tests;
