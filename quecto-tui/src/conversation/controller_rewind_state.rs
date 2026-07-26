use std::collections::HashMap;

use crate::components::select_list::SelectList;

/// Rewind flow state, grouped by owner (#997).
#[derive(Default)]
pub(crate) struct RewindFlow {
    /// Rewind selector shown after idle double-Escape lists prior user turns.
    pub(super) selector: Option<SelectList>,
    /// User-message content for each selectable rewind target.
    pub(super) selector_text_by_id: HashMap<String, String>,
    /// Last idle bare Escape used to detect double-Escape for rewind.
    pub(super) last_idle_escape: Option<tokio::time::Instant>,
    /// Locally-issued get_messages id for opening the rewind selector.
    pub(super) pending_open_id: Option<String>,
    /// Locally-issued rewind_to id awaiting acknowledgement.
    pub(super) pending_apply_id: Option<String>,
    /// Content of the selected user message, staged for the editor if rewind
    /// succeeds.
    pub(super) pending_apply_text: Option<String>,
    /// Monotonic client-local sequence for rewind correlation ids.
    pub(super) request_seq: u64,
}
