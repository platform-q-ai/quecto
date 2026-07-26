//! Shared suggestion-list state for the suggestion-backed surfaces: the
//! slash-command [`Autocomplete`](super::autocomplete::Autocomplete), the
//! [`FilesAutocomplete`](super::files_autocomplete::FilesAutocomplete) and
//! (since #997) the model selector. Each holds [`Suggestion`]s, tracks a
//! selected index, windows to a maximum height, and replaces the list while
//! preserving (or clamping) the selection; the components differ only by how
//! they *build* suggestions and style their rows.

use crate::components::autocomplete::{AutocompleteResult, Suggestion};
use crate::components::list_navigator::ListNavigator;
use crate::components::list_rows::{DescriptionMode, ListRow, render_windowed};
use crate::shell::keys::Key;

/// Shared selection/window state for a suggestion dropdown.
#[derive(Debug)]
pub struct SuggestionList {
    suggestions: Vec<Suggestion>,
    navigator: ListNavigator,
    max_visible: usize,
    active: bool,
}

impl SuggestionList {
    pub fn new(max_visible: usize) -> Self {
        Self {
            suggestions: Vec::new(),
            navigator: ListNavigator::new(),
            max_visible,
            active: false,
        }
    }

    /// Replace suggestions, preserving the selection if the list is unchanged.
    pub fn set_suggestions(&mut self, new: Vec<Suggestion>) {
        if !suggestions_match(&self.suggestions, &new) {
            self.navigator.reset();
        }
        self.set_suggestions_clamping(new);
    }

    /// Replace suggestions, CLAMPING the selection into the new range instead
    /// of resetting it — the model selector's historical filter-change
    /// semantics (#997): narrowing keeps the highlight on the last match.
    pub fn set_suggestions_clamping(&mut self, new: Vec<Suggestion>) {
        self.suggestions = new;
        self.active = !self.suggestions.is_empty();
        self.navigator.clamp(self.suggestions.len());
    }

    /// Shared dropdown key handling (#997): Up/Down navigate, Tab/Enter accept
    /// the selected value (only when `can_accept` — `@files` refuses its
    /// loading placeholder), Escape dismisses. Returns key-consumed.
    pub fn handle_key(
        &mut self,
        key: &Key,
        can_accept: bool,
        result: &mut AutocompleteResult,
    ) -> bool {
        if !self.active {
            return false;
        }
        match key {
            Key::Up => self.move_previous(),
            Key::Down => self.move_next(),
            Key::Tab | Key::Enter => {
                if can_accept && let Some(s) = self.selected_suggestion() {
                    *result = AutocompleteResult::Selected(s.value.clone());
                    self.close();
                }
            }
            Key::Escape => {
                *result = AutocompleteResult::Dismissed;
                self.close();
            }
            _ => return false,
        }
        true
    }

    /// Render this list through the shared row renderer (`list_rows`, #997);
    /// `to_row` builds each visible row's surface-specific label/decorations.
    pub fn render_rows(
        &self,
        width: usize,
        indent: &str,
        mode: DescriptionMode,
        to_row: impl Fn(&Suggestion) -> ListRow,
    ) -> Vec<String> {
        render_windowed(
            &self.suggestions,
            &self.navigator,
            self.max_visible,
            width,
            indent,
            mode,
            to_row,
        )
    }

    /// Clear suggestions and deactivate the dropdown.
    pub fn clear(&mut self) {
        self.active = false;
        self.suggestions.clear();
    }

    /// Hide the dropdown without discarding its suggestions (e.g. after select).
    pub fn close(&mut self) {
        self.active = false;
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// The configured maximum number of visible rows (the render window height).
    pub fn max_visible(&self) -> usize {
        self.max_visible
    }

    pub fn suggestions(&self) -> &[Suggestion] {
        &self.suggestions
    }

    pub fn len(&self) -> usize {
        self.suggestions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.suggestions.is_empty()
    }

    pub fn selected(&self) -> usize {
        self.navigator.selected()
    }

    pub fn selected_suggestion(&self) -> Option<&Suggestion> {
        self.suggestions.get(self.navigator.selected())
    }

    pub fn move_next(&mut self) {
        self.navigator.move_next(self.suggestions.len());
    }

    pub fn move_previous(&mut self) {
        self.navigator.move_previous(self.suggestions.len());
    }
}

/// Check if two suggestion lists have the same entries (compared by value).
fn suggestions_match(a: &[Suggestion], b: &[Suggestion]) -> bool {
    a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.value == y.value)
}

#[cfg(test)]
#[path = "suggestion_list_tests.rs"]
mod tests;
