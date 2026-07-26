use super::*;

impl SuggestionList {
    /// The visible window for the current selection (test observer).
    fn visible_range(&self) -> std::ops::Range<usize> {
        self.navigator
            .visible_range(self.suggestions.len(), self.max_visible)
    }
}

fn sugg(value: &str) -> Suggestion {
    Suggestion {
        value: value.to_string(),
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
fn clamping_set_keeps_selection_in_new_range() {
    let mut list = SuggestionList::new(5);
    list.set_suggestions(vec![sugg("a"), sugg("b"), sugg("c"), sugg("d")]);
    for _ in 0..3 {
        list.move_next();
    }
    assert_eq!(list.selected(), 3);
    list.set_suggestions_clamping(vec![sugg("a"), sugg("b")]);
    assert_eq!(list.selected(), 1, "selection clamps to the last new row");
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
