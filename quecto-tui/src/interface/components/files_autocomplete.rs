//! `@files` autocomplete — a popup of workspace file paths shown when the user
//! types an `@token` in the editor.
//!
//! Mirrors the slash-command [`Autocomplete`](super::autocomplete::Autocomplete)
//! (same navigation, selection, and render style) but triggers on an `@` at the
//! cursor and sources candidates from a workspace file list supplied by the app
//! event loop. Enumeration is intentionally not performed here: the UI component
//! only requests a load and renders a pending/loaded state.

use std::time::{Duration, Instant};

use crate::interface::component::Component;
use crate::interface::components::autocomplete::{AutocompleteResult, Suggestion};
use crate::interface::components::list_rows::{DescriptionMode, ListRow};
use crate::interface::components::suggestion_list::SuggestionList;
use crate::interface::fuzzy::fuzzy_filter_limited;
use crate::interface::keys::Key;

/// How long a loaded file list stays fresh before the next activation reloads
/// it — so files the agent creates mid-session eventually appear.
const CACHE_TTL: Duration = Duration::from_secs(30);

/// `@files` autocomplete dropdown.
#[derive(Debug)]
pub struct FilesAutocomplete {
    /// Workspace file paths (relative), supplied asynchronously by the app.
    files: Vec<String>,
    /// Files were injected (tests) and must never be reloaded from disk.
    injected: bool,
    /// When the list was last loaded; `None` until the first load.
    loaded_at: Option<Instant>,
    /// A background load has been requested and not yet completed.
    loading: bool,
    /// Latched request bit consumed by the app event loop.
    load_requested: bool,
    list: SuggestionList,
    result: AutocompleteResult,
    /// Byte offset of the `@` that begins the current token on the cursor line.
    token_start: Option<usize>,
    /// Last `(cursor_col, line)` seen by `update`, for skip-if-unchanged.
    /// Stored as separate fields so the unchanged-input fast path compares
    /// them directly instead of allocating a composite key every keystroke.
    last_cursor_col: Option<usize>,
    last_line: String,
}

impl FilesAutocomplete {
    /// Production constructor — the file list is requested lazily on the first
    /// `@` activation (never blocks construction, plain typing, or update).
    pub fn new(max_visible: usize) -> Self {
        Self {
            files: Vec::new(),
            injected: false,
            loaded_at: None,
            loading: false,
            load_requested: false,
            list: SuggestionList::new(max_visible),
            result: AutocompleteResult::Pending,
            token_start: None,
            last_cursor_col: None,
            last_line: String::new(),
        }
    }

    /// Construct with an explicit file list (no lazy loading) — for tests.
    pub fn with_files(files: Vec<String>, max_visible: usize) -> Self {
        let mut s = Self::new(max_visible);
        s.files = files;
        s.injected = true;
        s.loaded_at = Some(Instant::now());
        s
    }

    /// Recompute suggestions for the `@token` ending at `cursor_col` on `line`.
    /// Deactivates when there is no `@token` at the cursor.
    pub fn update(&mut self, line: &str, cursor_col: usize) {
        if self.last_cursor_col == Some(cursor_col) && self.last_line == line {
            return;
        }
        self.last_cursor_col = Some(cursor_col);
        self.last_line.clear();
        self.last_line.push_str(line);

        let Some((start, prefix)) = at_token(line, cursor_col) else {
            self.deactivate();
            return;
        };
        self.token_start = Some(start);
        self.request_load_if_needed();

        if self.loading && self.files.is_empty() {
            self.list.set_suggestions(vec![Suggestion {
                value: "loading files…".to_string(),
                description: String::new(),
            }]);
            return;
        }

        // Bound stored matches; rendering windows to `max_visible` anyway.
        let new: Vec<Suggestion> =
            fuzzy_filter_limited(&self.files, prefix, self.list.max_visible() * 4, |f| {
                f.as_str()
            })
            .into_iter()
            .map(|f| Suggestion {
                value: f.clone(),
                description: String::new(),
            })
            .collect();
        self.list.set_suggestions(new);
    }

    /// Apply files loaded by the app event loop's background worker.
    pub fn apply_loaded_files(&mut self, files: Vec<String>) {
        self.files = files;
        self.loaded_at = Some(Instant::now());
        self.loading = false;
        self.load_requested = false;
        // Force the next update to recompute the currently visible token with
        // the newly supplied files, even if the editor text did not change.
        self.last_cursor_col = None;
        self.last_line.clear();
    }

    /// Test helper: mark the cache stale without sleeping.
    pub fn mark_loaded_at_for_test(&mut self, loaded_at: Instant) {
        self.loaded_at = Some(loaded_at);
        self.injected = false;
    }

    /// Consume a pending background-load request.
    pub fn take_load_request(&mut self) -> bool {
        std::mem::take(&mut self.load_requested)
    }

    /// Request an async file-list load when stale. Injected test lists never
    /// reload. This method deliberately does not enumerate files.
    fn request_load_if_needed(&mut self) {
        let age = self.loaded_at.map(|t| t.elapsed());
        if !should_reload(self.injected, age, CACHE_TTL) || self.loading {
            return;
        }
        self.loading = true;
        self.load_requested = true;
    }

    fn deactivate(&mut self) {
        self.list.clear();
        self.token_start = None;
    }

    /// Whether the dropdown is currently visible.
    pub fn is_active(&self) -> bool {
        self.list.is_active()
    }

    /// Dismiss the dropdown and force the next `update` to re-evaluate.
    pub fn dismiss(&mut self) {
        self.deactivate();
        self.last_cursor_col = None;
        self.last_line.clear();
    }

    /// Byte offset of the `@` starting the active token (for insertion).
    pub fn token_start(&self) -> Option<usize> {
        self.token_start
    }

    /// Take the result of the last interaction.
    pub fn take_result(&mut self) -> AutocompleteResult {
        std::mem::replace(&mut self.result, AutocompleteResult::Pending)
    }

    /// Current suggestion values (test observer).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn suggestion_values(&self) -> Vec<String> {
        self.list
            .suggestions()
            .iter()
            .map(|suggestion| suggestion.value.clone())
            .collect()
    }

    /// Number of current suggestions (test observer).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn suggestion_count(&self) -> usize {
        self.list.suggestions().len()
    }
}

/// Find the `@token` ending at `cursor_col`: the last `@` before the cursor that
/// sits at line start or after whitespace, with no whitespace up to the cursor.
/// Returns `(byte offset of '@', text after '@')`.
fn at_token(line: &str, cursor_col: usize) -> Option<(usize, &str)> {
    let cursor = cursor_col.min(line.len());
    if !line.is_char_boundary(cursor) {
        return None;
    }
    let before = &line[..cursor];
    let at = before.rfind('@')?;
    // `@` must start the line or follow whitespace (so `user@host` doesn't fire).
    if at > 0 {
        let prev = before[..at].chars().next_back()?;
        if !prev.is_whitespace() {
            return None;
        }
    }
    let prefix = &before[at + 1..];
    if prefix.chars().any(char::is_whitespace) {
        return None;
    }
    Some((at, prefix))
}

/// Whether the file cache should be (re)loaded: never for injected test lists;
/// otherwise when it has never loaded or its age has reached the TTL.
fn should_reload(injected: bool, age: Option<Duration>, ttl: Duration) -> bool {
    if injected {
        return false;
    }
    match age {
        None => true,
        Some(a) => a >= ttl,
    }
}

impl Component for FilesAutocomplete {
    fn render(&mut self, width: usize) -> Vec<String> {
        if !self.list.is_active() || self.list.is_empty() {
            return vec![];
        }

        // Shared row renderer (#997). Only the empty-list loading placeholder
        // renders dim (bare label, no `@`); real rows — even while a STALE
        // list reloads in the background — keep `@`, the arrow and the accent.
        let placeholder = self.loading && self.files.is_empty();
        let mode = DescriptionMode::AlignedCached { label_width: 0 };
        self.list.render_rows(width, "", mode, |s| ListRow {
            dim_label: placeholder,
            ..ListRow::plain(if placeholder {
                s.value.clone()
            } else {
                format!("@{}", s.value)
            })
        })
    }

    fn handle_input(&mut self, key: &Key) -> bool {
        // Tab/Enter must not accept the loading placeholder row.
        let can_accept = !self.loading || !self.files.is_empty();
        self.list.handle_key(key, can_accept, &mut self.result)
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
#[path = "files_autocomplete_tests.rs"]
mod tests;
