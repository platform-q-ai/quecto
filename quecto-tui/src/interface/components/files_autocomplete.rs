//! `@files` autocomplete — a popup of workspace file paths shown when the user
//! types an `@token` in the editor.
//!
//! Mirrors the slash-command [`Autocomplete`](super::autocomplete::Autocomplete)
//! (same navigation, selection, and render style) but triggers on an `@` at the
//! cursor and sources candidates from the workspace file list — loaded lazily on
//! first activation via `git ls-files` (tracked + untracked-not-ignored),
//! falling back to a bounded filesystem walk when git is unavailable.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::infrastructure::workspace_files::list_workspace_files;
use crate::interface::component::Component;
use crate::interface::components::autocomplete::{AutocompleteResult, Suggestion};
use crate::interface::fuzzy::fuzzy_filter;
use crate::interface::keys::Key;
use crate::interface::theme;
use crate::interface::utils::truncate_to_width;

/// How long a loaded file list stays fresh before the next activation reloads
/// it — so files the agent creates mid-session eventually appear.
const CACHE_TTL: Duration = Duration::from_secs(30);

/// `@files` autocomplete dropdown.
#[derive(Debug)]
pub struct FilesAutocomplete {
    /// Workspace file paths (relative), loaded lazily on first activation.
    files: Vec<String>,
    /// Files were injected (tests) and must never be reloaded from disk.
    injected: bool,
    /// When the list was last loaded; `None` until the first load.
    loaded_at: Option<Instant>,
    suggestions: Vec<Suggestion>,
    selected: usize,
    max_visible: usize,
    active: bool,
    result: AutocompleteResult,
    /// Byte offset of the `@` that begins the current token on the cursor line.
    token_start: Option<usize>,
    /// Last (cursor, line) seen by `update`, for skip-if-unchanged.
    last_key: String,
}

impl FilesAutocomplete {
    /// Production constructor — the file list is loaded lazily from the cwd on
    /// the first `@` activation (never blocks construction or plain typing).
    pub fn new(max_visible: usize) -> Self {
        Self {
            files: Vec::new(),
            injected: false,
            loaded_at: None,
            suggestions: Vec::new(),
            selected: 0,
            max_visible,
            active: false,
            result: AutocompleteResult::Pending,
            token_start: None,
            last_key: String::new(),
        }
    }

    /// Construct with an explicit file list (no lazy loading) — for tests.
    pub fn with_files(files: Vec<String>, max_visible: usize) -> Self {
        let mut s = Self::new(max_visible);
        s.files = files;
        s.injected = true;
        s
    }

    /// Recompute suggestions for the `@token` ending at `cursor_col` on `line`.
    /// Deactivates when there is no `@token` at the cursor.
    pub fn update(&mut self, line: &str, cursor_col: usize) {
        let key = format!("{cursor_col}\u{0}{line}");
        if key == self.last_key {
            return;
        }
        self.last_key = key;

        let Some((start, prefix)) = at_token(line, cursor_col) else {
            self.deactivate();
            return;
        };
        self.token_start = Some(start);
        self.ensure_loaded();

        let new: Vec<Suggestion> = fuzzy_filter(&self.files, prefix, |f| f.as_str())
            .into_iter()
            // Bound the work; the render windows to `max_visible` anyway.
            .take(self.max_visible * 4)
            .map(|f| Suggestion {
                value: f.clone(),
                label: f.clone(),
                description: String::new(),
            })
            .collect();
        self.set_suggestions(new);
    }

    /// Load the workspace file list from the cwd on first use, and reload it
    /// when the cache has gone stale (so files created mid-session appear).
    /// Injected (test) lists are never reloaded.
    fn ensure_loaded(&mut self) {
        let age = self.loaded_at.map(|t| t.elapsed());
        if !should_reload(self.injected, age, CACHE_TTL) {
            return;
        }
        let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
        self.files = list_workspace_files(&cwd);
        self.loaded_at = Some(Instant::now());
    }

    fn set_suggestions(&mut self, new: Vec<Suggestion>) {
        if !suggestions_match(&self.suggestions, &new) {
            self.selected = 0;
        }
        self.suggestions = new;
        self.active = !self.suggestions.is_empty();
        if self.selected >= self.suggestions.len() && !self.suggestions.is_empty() {
            self.selected = self.suggestions.len() - 1;
        }
    }

    fn deactivate(&mut self) {
        self.active = false;
        self.suggestions.clear();
        self.token_start = None;
    }

    /// Whether the dropdown is currently visible.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Dismiss the dropdown and force the next `update` to re-evaluate.
    pub fn dismiss(&mut self) {
        self.deactivate();
        self.last_key.clear();
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

fn suggestions_match(a: &[Suggestion], b: &[Suggestion]) -> bool {
    a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.value == y.value)
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
        if !self.active || self.suggestions.is_empty() {
            return vec![];
        }

        let mut lines = Vec::new();
        let total = self.suggestions.len();
        let visible = total.min(self.max_visible);
        let start = if self.selected >= visible {
            (self.selected + 1).saturating_sub(visible)
        } else {
            0
        };
        let end = (start + visible).min(total);

        for i in start..end {
            let s = &self.suggestions[i];
            let is_sel = i == self.selected;
            let prefix = if is_sel { "→ " } else { "  " };
            let name = if is_sel {
                theme::accent(&format!("@{}", s.label))
            } else {
                format!("@{}", s.label)
            };
            lines.push(truncate_to_width(&format!("{prefix}{name}"), width, None));
        }

        if start > 0 || end < total {
            lines.push(theme::dim(&format!("  ({}/{})", self.selected + 1, total)));
        }

        lines
    }

    fn handle_input(&mut self, key: &Key) -> bool {
        if !self.active {
            return false;
        }
        match key {
            Key::Up => {
                if self.selected == 0 {
                    self.selected = self.suggestions.len().saturating_sub(1);
                } else {
                    self.selected -= 1;
                }
                true
            }
            Key::Down => {
                if self.selected >= self.suggestions.len().saturating_sub(1) {
                    self.selected = 0;
                } else {
                    self.selected += 1;
                }
                true
            }
            Key::Tab | Key::Enter => {
                if let Some(s) = self.suggestions.get(self.selected) {
                    self.result = AutocompleteResult::Selected(s.value.clone());
                    self.active = false;
                }
                true
            }
            Key::Escape => {
                self.result = AutocompleteResult::Dismissed;
                self.active = false;
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
        assert_eq!(f.suggestions.len(), 4, "@ alone lists all files");
        assert_eq!(f.token_start(), Some(0));
    }

    #[test]
    fn fuzzy_filters_on_prefix() {
        let mut f = fa();
        f.update("@main", 5);
        assert!(f.is_active());
        assert!(
            f.suggestions[0].value.contains("main.rs"),
            "best match should be main.rs: {:?}",
            f.suggestions
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
        assert!(f.suggestions[0].value.contains("lib.rs"));
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
        assert_eq!(f.selected, 0);
        f.handle_input(&Key::Down);
        assert_eq!(f.selected, 1);
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
