//! Shared suggestion-list state for dropdown components.
//!
//! Both the slash-command [`Autocomplete`](super::autocomplete::Autocomplete)
//! and the [`FilesAutocomplete`](super::files_autocomplete::FilesAutocomplete)
//! are near-identical popups: they hold a list of [`Suggestion`]s, track a
//! selected index, window the list to a maximum height, and replace the list
//! while preserving the selection when it is unchanged. That shared state lives
//! here so the two components differ only by how they *build* suggestions and
//! how they *render* a row (prefix, description column, loading guard).

use std::ops::Range;

use crate::interface::components::autocomplete::Suggestion;
use crate::interface::components::list_navigator::ListNavigator;

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
        self.suggestions = new;
        self.active = !self.suggestions.is_empty();
        self.navigator.clamp(self.suggestions.len());
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

    /// The visible window over the suggestion list for the current selection.
    pub fn visible_range(&self) -> Range<usize> {
        self.navigator
            .visible_range(self.suggestions.len(), self.max_visible)
    }
}

/// Check if two suggestion lists have the same entries (compared by value).
fn suggestions_match(a: &[Suggestion], b: &[Suggestion]) -> bool {
    a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.value == y.value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sugg(value: &str) -> Suggestion {
        Suggestion {
            value: value.to_string(),
            label: value.to_string(),
            description: String::new(),
        }
    }

    #[test]
    fn set_suggestions_activates_and_windows() {
        let mut list = SuggestionList::new(2);
        list.set_suggestions(vec![sugg("a"), sugg("b"), sugg("c")]);
        assert!(list.is_active());
        assert_eq!(list.len(), 3);
        assert_eq!(list.visible_range(), 0..2);
    }

    #[test]
    fn empty_set_deactivates() {
        let mut list = SuggestionList::new(5);
        list.set_suggestions(vec![sugg("a")]);
        assert!(list.is_active());
        list.set_suggestions(vec![]);
        assert!(!list.is_active());
        assert!(list.is_empty());
    }

    #[test]
    fn unchanged_set_preserves_selection() {
        let mut list = SuggestionList::new(5);
        list.set_suggestions(vec![sugg("a"), sugg("b"), sugg("c")]);
        list.move_next();
        assert_eq!(list.selected(), 1);
        // Re-setting the same values (compared by value) keeps selection.
        list.set_suggestions(vec![sugg("a"), sugg("b"), sugg("c")]);
        assert_eq!(list.selected(), 1);
    }

    #[test]
    fn changed_set_resets_selection() {
        let mut list = SuggestionList::new(5);
        list.set_suggestions(vec![sugg("a"), sugg("b"), sugg("c")]);
        list.move_next();
        list.move_next();
        assert_eq!(list.selected(), 2);
        list.set_suggestions(vec![sugg("x")]);
        assert_eq!(list.selected(), 0);
    }

    #[test]
    fn move_wraps_via_navigator() {
        let mut list = SuggestionList::new(5);
        list.set_suggestions(vec![sugg("a"), sugg("b")]);
        list.move_previous();
        assert_eq!(list.selected(), 1);
        list.move_next();
        assert_eq!(list.selected(), 0);
    }

    #[test]
    fn close_hides_but_keeps_suggestions() {
        let mut list = SuggestionList::new(5);
        list.set_suggestions(vec![sugg("a")]);
        list.close();
        assert!(!list.is_active());
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn selected_suggestion_returns_current_entry() {
        let mut list = SuggestionList::new(5);
        list.set_suggestions(vec![sugg("a"), sugg("b")]);
        list.move_next();
        assert_eq!(
            list.selected_suggestion().map(|s| s.value.as_str()),
            Some("b")
        );
    }
}
