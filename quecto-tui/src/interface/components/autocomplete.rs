//! Autocomplete dropdown — renders below the editor when typing `/`.
//!
//! Provides fuzzy-matched suggestions for slash commands and model names.

use crate::interface::component::Component;
use crate::interface::components::suggestion_list::SuggestionList;
use crate::interface::fuzzy::fuzzy_filter;
use crate::interface::keys::Key;
use crate::interface::theme;
use crate::interface::utils::truncate_to_width;

/// A slash command definition.
#[derive(Debug, Clone)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
}

/// Autocomplete suggestion.
#[derive(Debug, Clone)]
pub struct Suggestion {
    pub value: String,
    pub label: String,
    pub description: String,
}

/// Result of an autocomplete interaction.
#[derive(Debug, Clone, PartialEq)]
pub enum AutocompleteResult {
    /// User selected a suggestion.
    Selected(String),
    /// User dismissed the autocomplete.
    Dismissed,
    /// Still browsing.
    Pending,
}

/// Autocomplete dropdown component.
pub struct Autocomplete {
    commands: Vec<SlashCommand>,
    list: SuggestionList,
    result: AutocompleteResult,
    /// Last text passed to update(), for skip-if-unchanged optimization.
    last_update_text: String,
}

impl Autocomplete {
    pub fn new(commands: Vec<SlashCommand>, max_visible: usize) -> Self {
        Self {
            commands,
            list: SuggestionList::new(max_visible),
            result: AutocompleteResult::Pending,
            last_update_text: String::new(),
        }
    }

    /// Update suggestions based on current editor text.
    ///
    /// Activates autocomplete when text starts with `/` and has at least one
    /// character after it. Deactivates when text doesn't match.
    pub fn update(&mut self, text: &str) {
        // Skip if text hasn't changed — avoids unnecessary allocation.
        if text == self.last_update_text {
            return;
        }
        self.last_update_text = text.to_string();

        let trimmed = text.trim();

        if !trimmed.starts_with('/') || trimmed.len() < 2 {
            if trimmed == "/" {
                // Show all commands.
                let new: Vec<Suggestion> = self
                    .commands
                    .iter()
                    .map(|c| Suggestion {
                        value: format!("/{}", c.name),
                        label: c.name.clone(),
                        description: c.description.clone(),
                    })
                    .collect();
                self.list.set_suggestions(new);
            } else {
                self.list.clear();
            }
            return;
        }

        // Extract the command prefix after `/`.
        let prefix = &trimmed[1..];
        // Don't autocomplete if there's a space (command has args).
        if prefix.contains(' ') {
            self.list.clear();
            return;
        }

        // Fuzzy filter commands.
        let filtered = fuzzy_filter(&self.commands, prefix, |c| &c.name);
        let new: Vec<Suggestion> = filtered
            .into_iter()
            .map(|c| Suggestion {
                value: format!("/{}", c.name),
                label: c.name.clone(),
                description: c.description.clone(),
            })
            .collect();
        self.list.set_suggestions(new);
    }

    /// Whether the autocomplete dropdown is currently visible.
    pub fn is_active(&self) -> bool {
        self.list.is_active()
    }

    /// Dismiss the autocomplete.
    pub fn dismiss(&mut self) {
        self.list.clear();
    }

    /// Take the result of the autocomplete interaction.
    pub fn take_result(&mut self) -> AutocompleteResult {
        std::mem::replace(&mut self.result, AutocompleteResult::Pending)
    }
}

impl Component for Autocomplete {
    fn render(&mut self, width: usize) -> Vec<String> {
        if !self.list.is_active() || self.list.is_empty() {
            return vec![];
        }

        let mut lines = Vec::new();
        let total = self.list.len();
        let range = self.list.visible_range();
        let start = range.start;
        let end = range.end;

        for i in start..end {
            let s = &self.list.suggestions()[i];
            let is_sel = i == self.list.selected();
            let prefix = if is_sel { "→ " } else { "  " };
            let name = if is_sel {
                theme::accent(&format!("/{}", s.label))
            } else {
                format!("/{}", s.label)
            };
            let desc = theme::dim(&s.description);
            let line = format!("{}{}  {}", prefix, name, desc);
            lines.push(truncate_to_width(&line, width, None));
        }

        if start > 0 || end < total {
            lines.push(theme::dim(&format!(
                "  ({}/{})",
                self.list.selected() + 1,
                total
            )));
        }

        lines
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
                if let Some(s) = self.list.selected_suggestion() {
                    self.result = AutocompleteResult::Selected(s.value.clone());
                    self.list.close();
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
        assert_eq!(ac.list.suggestions()[0].label, "model");
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
        let first = ac.list.suggestions()[0].label.clone();
        ac.handle_input(&Key::Down);
        let second = ac.list.suggestions()[ac.list.selected()].label.clone();
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
}
