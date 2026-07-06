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
use crate::interface::components::list_rows::{
    DescriptionMode, ListRow, RowStyle, render_list_rows,
};
use crate::interface::components::suggestion_list::SuggestionList;
use crate::interface::fuzzy::fuzzy_filter;
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
            self.list.set_suggestions(vec![loading_suggestion()]);
            return;
        }

        let new: Vec<Suggestion> = fuzzy_filter(&self.files, prefix, |f| f.as_str())
            .into_iter()
            // Bound the work; the render windows to `max_visible` anyway.
            .take(self.list.max_visible() * 4)
            .map(|f| Suggestion {
                value: f.clone(),
                label: f.clone(),
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

    /// Whether a background load is currently pending (tests only).
    #[cfg(test)]
    pub fn is_loading(&self) -> bool {
        self.loading
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
}

fn loading_suggestion() -> Suggestion {
    Suggestion {
        value: String::new(),
        label: "loading files…".to_string(),
        description: String::new(),
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
        // renders dim (and keeps its bare label, no `@`); real rows — even
        // while a STALE list reloads in the background — keep the `@` sigil,
        // the selection arrow and the accent.
        let placeholder = self.loading && self.files.is_empty();
        let rows: Vec<ListRow> = self.list.suggestions()[self.list.visible_range()]
            .iter()
            .map(|s| ListRow {
                label: if placeholder {
                    s.label.clone()
                } else {
                    format!("@{}", s.label)
                },
                description: None,
                marker: "",
                dim_label: placeholder,
            })
            .collect();
        let style = RowStyle {
            indent: "",
            description: DescriptionMode::Inline,
        };
        render_list_rows(
            &rows,
            self.list.navigator(),
            self.list.len(),
            self.list.max_visible(),
            width,
            &style,
        )
    }

    fn handle_input(&mut self, key: &Key) -> bool {
        if !self.list.is_active() {
            return false;
        }
        match key {
            Key::Up => {
                self.list.move_previous();
                true
            }
            Key::Down => {
                self.list.move_next();
                true
            }
            Key::Tab | Key::Enter => {
                if !self.loading || !self.files.is_empty() {
                    if let Some(s) = self.list.selected_suggestion() {
                        self.result = AutocompleteResult::Selected(s.value.clone());
                        self.list.close();
                    }
                }
                true
            }
            Key::Escape => {
                self.result = AutocompleteResult::Dismissed;
                self.list.close();
                true
            }
            _ => false,
        }
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fa() -> FilesAutocomplete {
        FilesAutocomplete::with_files(
            vec![
                "src/main.rs".to_string(),
                "src/lib.rs".to_string(),
                "README.md".to_string(),
                "docs/workflow.md".to_string(),
            ],
            5,
        )
    }

    #[test]
    fn activates_on_at() {
        let mut f = fa();
        f.update("@", 1);
        assert!(f.is_active());
        assert_eq!(f.list.len(), 4, "@ alone lists all files");
        assert_eq!(f.token_start(), Some(0));
    }

    #[test]
    fn first_activation_requests_load_and_shows_loading_without_enumerating() {
        let mut f = FilesAutocomplete::new(5);
        f.update("@", 1);
        assert!(f.is_active());
        assert!(f.is_loading());
        assert!(f.take_load_request());
        assert_eq!(f.list.len(), 1);
        assert_eq!(f.list.suggestions()[0].label, "loading files…");
    }

    #[test]
    fn loaded_files_replace_loading_state_on_next_update() {
        let mut f = FilesAutocomplete::new(5);
        f.update("@main", 5);
        assert!(f.take_load_request());
        f.apply_loaded_files(vec!["src/main.rs".to_string(), "src/lib.rs".to_string()]);
        f.update("@main", 5);
        assert!(!f.is_loading());
        assert!(f.is_active());
        assert_eq!(f.list.suggestions()[0].value, "src/main.rs");
    }

    #[test]
    fn stale_activation_requests_reload_without_dropping_loaded_suggestions() {
        let mut f = FilesAutocomplete::with_files(vec!["src/main.rs".to_string()], 5);
        f.mark_loaded_at_for_test(Instant::now() - CACHE_TTL - Duration::from_secs(1));
        f.update("@", 1);
        assert!(f.take_load_request());
        assert!(f.is_loading());
        assert!(f.is_active());
        assert_eq!(f.list.suggestions()[0].value, "src/main.rs");
    }

    #[test]
    fn fuzzy_filters_on_prefix() {
        let mut f = fa();
        f.update("@main", 5);
        assert!(f.is_active());
        assert!(
            f.list.suggestions()[0].value.contains("main.rs"),
            "best match should be main.rs: {:?}",
            f.list.suggestions()
        );
    }

    #[test]
    fn inactive_without_at() {
        let mut f = fa();
        f.update("hello world", 11);
        assert!(!f.is_active());
        assert_eq!(f.token_start(), None);
    }

    #[test]
    fn deactivates_when_token_has_whitespace() {
        let mut f = fa();
        f.update("@src done", 9);
        assert!(!f.is_active(), "a space ends the @token");
    }

    #[test]
    fn does_not_fire_on_email_like_at() {
        let mut f = fa();
        // `@` preceded by a non-space char (e.g. an email) must not trigger.
        f.update("user@host", 9);
        assert!(!f.is_active());
    }

    #[test]
    fn fires_mid_line_after_space() {
        let mut f = fa();
        f.update("see @src/li", 11);
        assert!(f.is_active());
        assert_eq!(f.token_start(), Some(4));
        assert!(f.list.suggestions()[0].value.contains("lib.rs"));
    }

    #[test]
    fn tab_selects_full_path() {
        let mut f = fa();
        f.update("@main", 5);
        f.handle_input(&Key::Tab);
        match f.take_result() {
            AutocompleteResult::Selected(v) => assert_eq!(v, "src/main.rs"),
            other => panic!("expected Selected, got {other:?}"),
        }
    }

    #[test]
    fn tab_does_not_select_loading_placeholder() {
        let mut f = FilesAutocomplete::new(5);
        f.update("@", 1);
        f.handle_input(&Key::Tab);
        assert_eq!(f.take_result(), AutocompleteResult::Pending);
        assert!(f.is_active());
    }

    #[test]
    fn escape_dismisses() {
        let mut f = fa();
        f.update("@", 1);
        f.handle_input(&Key::Escape);
        assert_eq!(f.take_result(), AutocompleteResult::Dismissed);
        assert!(!f.is_active());
    }

    #[test]
    fn navigate_down_changes_selection() {
        let mut f = fa();
        f.update("@", 1);
        assert_eq!(f.list.selected(), 0);
        f.handle_input(&Key::Down);
        assert_eq!(f.list.selected(), 1);
    }

    #[test]
    fn renders_active_suggestions() {
        let mut f = fa();
        f.update("@", 1);
        let lines = f.render(60);
        assert!(!lines.is_empty());
        let plain: String = lines
            .join("\n")
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .collect();
        assert!(plain.contains("@src/main.rs"), "should list files: {plain}");
    }

    #[test]
    fn renders_nothing_when_inactive() {
        let mut f = fa();
        assert!(f.render(60).is_empty());
    }

    #[test]
    fn cache_reload_policy() {
        let ttl = Duration::from_secs(30);
        // Injected (test) lists never reload, regardless of age.
        assert!(!should_reload(true, None, ttl));
        assert!(!should_reload(true, Some(Duration::from_secs(999)), ttl));
        // Production: load when never loaded, keep while fresh, reload when stale.
        assert!(should_reload(false, None, ttl));
        assert!(!should_reload(false, Some(Duration::from_secs(5)), ttl));
        assert!(should_reload(false, Some(Duration::from_secs(31)), ttl));
    }
}
