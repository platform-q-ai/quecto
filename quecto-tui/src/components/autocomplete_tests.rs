use super::*;

fn test_commands() -> Vec<SlashCommand> {
    vec![
        SlashCommand {
            name: "model".to_string(),
            description: "Select model".to_string(),
        },
        SlashCommand {
            name: "clear".to_string(),
            description: "Clear history".to_string(),
        },
        SlashCommand {
            name: "quit".to_string(),
            description: "Exit TUI".to_string(),
        },
        SlashCommand {
            name: "settings".to_string(),
            description: "Open settings".to_string(),
        },
    ]
}

#[test]
fn activates_on_slash() {
    let mut ac = Autocomplete::new(test_commands(), 5);
    ac.update("/");
    assert!(ac.is_active());
    assert_eq!(ac.list.len(), 4);
}

#[test]
fn filters_on_prefix() {
    let mut ac = Autocomplete::new(test_commands(), 5);
    ac.update("/mo");
    assert!(ac.is_active());
    assert_eq!(ac.list.len(), 1);
    assert_eq!(ac.list.suggestions()[0].value, "/model");
}

#[test]
fn inactive_when_no_slash() {
    let mut ac = Autocomplete::new(test_commands(), 5);
    ac.update("hello");
    assert!(!ac.is_active());
}

#[test]
fn inactive_when_command_has_args() {
    let mut ac = Autocomplete::new(test_commands(), 5);
    ac.update("/model claude");
    assert!(!ac.is_active());
}

#[test]
fn tab_selects() {
    let mut ac = Autocomplete::new(test_commands(), 5);
    ac.update("/mo");
    ac.handle_input(&Key::Tab);
    assert_eq!(
        ac.take_result(),
        AutocompleteResult::Selected("/model".to_string())
    );
}

#[test]
fn escape_dismisses() {
    let mut ac = Autocomplete::new(test_commands(), 5);
    ac.update("/");
    ac.handle_input(&Key::Escape);
    assert_eq!(ac.take_result(), AutocompleteResult::Dismissed);
    assert!(!ac.is_active());
}

#[test]
fn navigate_down_up() {
    let mut ac = Autocomplete::new(test_commands(), 5);
    ac.update("/");
    let first = ac.list.suggestions()[0].value.clone();
    ac.handle_input(&Key::Down);
    let second = ac.list.suggestions()[ac.list.selected()].value.clone();
    assert_ne!(first, second);
}

#[test]
fn renders_when_active() {
    let mut ac = Autocomplete::new(test_commands(), 5);
    ac.update("/");
    let lines = ac.render(60);
    assert!(!lines.is_empty());
    let joined: String = lines.join("\n");
    let plain: String = joined
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect();
    assert!(plain.contains("/model"), "should contain /model: {}", plain);
}

#[test]
fn renders_nothing_when_inactive() {
    let mut ac = Autocomplete::new(test_commands(), 5);
    let lines = ac.render(60);
    assert!(lines.is_empty());
}

// --- Autocomplete Enter contract tests (#471) ---

// --- Arrow navigation tests (#477) ---

#[test]
fn down_arrow_advances_sequentially() {
    let mut ac = Autocomplete::new(test_commands(), 5);
    ac.update("/");
    assert_eq!(ac.list.selected(), 0);
    ac.handle_input(&Key::Down);
    assert_eq!(ac.list.selected(), 1);
    ac.handle_input(&Key::Down);
    assert_eq!(ac.list.selected(), 2);
    ac.handle_input(&Key::Down);
    assert_eq!(ac.list.selected(), 3);
}

#[test]
fn up_arrow_goes_backwards() {
    let mut ac = Autocomplete::new(test_commands(), 5);
    ac.update("/");
    ac.handle_input(&Key::Down);
    ac.handle_input(&Key::Down);
    assert_eq!(ac.list.selected(), 2);
    ac.handle_input(&Key::Up);
    assert_eq!(ac.list.selected(), 1);
}

#[test]
fn update_same_text_preserves_selection() {
    let mut ac = Autocomplete::new(test_commands(), 5);
    ac.update("/");
    ac.handle_input(&Key::Down);
    ac.handle_input(&Key::Down);
    assert_eq!(ac.list.selected(), 2);
    // Calling update with same text should NOT reset selection.
    ac.update("/");
    assert_eq!(
        ac.list.selected(),
        2,
        "update with same text should preserve selection"
    );
}

#[test]
fn update_different_text_resets_selection() {
    let mut ac = Autocomplete::new(test_commands(), 5);
    ac.update("/");
    ac.handle_input(&Key::Down);
    ac.handle_input(&Key::Down);
    assert_eq!(ac.list.selected(), 2);
    // Changing text should reset selection.
    ac.update("/mo");
    assert_eq!(
        ac.list.selected(),
        0,
        "update with new text should reset selection"
    );
}

#[test]
fn tab_select_returns_full_command() {
    // Tab on a partial match should return the full command text.
    let mut ac = Autocomplete::new(test_commands(), 5);
    ac.update("/mo");
    assert!(ac.is_active());
    ac.handle_input(&Key::Tab);
    match ac.take_result() {
        AutocompleteResult::Selected(value) => {
            assert_eq!(value, "/model", "Tab should return the full command");
        }
        other => panic!("expected Selected, got {:?}", other),
    }
}

#[test]
fn tab_select_returns_value_for_enter_submit_path() {
    // The Enter path in app.rs simulates Tab-to-accept then submits.
    // This test verifies the autocomplete returns the right value.
    let mut ac = Autocomplete::new(test_commands(), 5);
    ac.update("/qu");
    assert!(ac.is_active());
    ac.handle_input(&Key::Tab);
    let result = ac.take_result();
    match result {
        AutocompleteResult::Selected(value) => {
            assert_eq!(value, "/quit");
        }
        other => panic!("expected Selected, got {:?}", other),
    }
}

#[test]
fn dismiss_clears_but_selected_accept_retains_suggestions() {
    let mut ac = Autocomplete::new(test_commands(), 5);
    ac.update("/");
    assert_eq!(ac.selected_index(), 0);
    assert_eq!(ac.selected_value(), Some("/model".to_string()));
    assert_eq!(ac.suggestion_count(), 4);

    ac.handle_input(&Key::Enter);
    assert_eq!(
        ac.take_result(),
        AutocompleteResult::Selected("/model".to_string())
    );
    assert_eq!(
        ac.suggestion_count(),
        4,
        "accepting closes without clearing cached suggestions"
    );

    ac.dismiss();
    assert_eq!(ac.suggestion_count(), 0);
    assert!(!ac.is_active());
}

#[test]
fn no_match_is_inactive_and_enter_is_not_consumed() {
    let mut ac = Autocomplete::new(test_commands(), 5);
    ac.update("/zz");

    assert!(!ac.is_active());
    assert_eq!(ac.suggestion_count(), 0);
    assert!(!ac.handle_input(&Key::Enter));
    assert_eq!(ac.take_result(), AutocompleteResult::Pending);
}
