use crate::components::select_list::SelectList;

/// Rewind flow state, grouped by owner (#997).
#[derive(Default)]
pub(crate) struct RewindFlow {
    /// Rewind selector shown after idle double-Escape lists prior user turns.
    pub(super) selector: Option<SelectList>,
    /// Last idle bare Escape used to detect double-Escape for rewind.
    pub(super) last_idle_escape: Option<tokio::time::Instant>,
    /// Locally-issued get_messages id for opening the rewind selector.
    pub(super) pending_open_id: Option<String>,
    /// Locally-issued get_message id awaiting the selected message's full body.
    pub(super) pending_load_id: Option<String>,
    /// Selected message id awaiting rewind after its full body is loaded.
    pub(super) pending_apply_message_id: Option<String>,
    /// Accumulated selected-message content while loading paged rewind text.
    pub(super) pending_load_content: String,
    /// Next expected byte offset for the selected-message rewind load.
    pub(super) pending_load_offset: usize,
    /// Locally-issued rewind_to id awaiting acknowledgement.
    pub(super) pending_apply_id: Option<String>,
    /// Editor contents when rewind apply was sent; protects user edits typed
    /// while the rewind command is in flight.
    pub(super) pending_apply_editor_baseline: Option<String>,
    /// Content of the selected user message, staged for the editor if rewind
    /// succeeds.
    pub(super) pending_apply_text: Option<String>,
    /// Monotonic client-local sequence for rewind correlation ids.
    pub(super) request_seq: u64,
}
