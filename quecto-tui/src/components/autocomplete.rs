//! Autocomplete dropdown — renders below the editor when typing `/`.
//!
//! Provides fuzzy-matched suggestions for slash commands and model names.

use crate::component::Component;
use crate::fuzzy::fuzzy_filter;
use crate::keys::Key;
use crate::theme;
use crate::utils::truncate_to_width;

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
    suggestions: Vec<Suggestion>,
    selected: usize,
    max_visible: usize,
    active: bool,
    result: AutocompleteResult,
}

impl Autocomplete {
    pub fn new(commands: Vec<SlashCommand>, max_visible: usize) -> Self {
        Self {
            commands,
            suggestions: Vec::new(),
            selected: 0,
            max_visible,
            active: false,
            result: AutocompleteResult::Pending,
        }
    }

    /// Update suggestions based on current editor text.
    ///
    /// Activates autocomplete when text starts with `/` and has at least one
    /// character after it. Deactivates when text doesn't match.
    pub fn update(&mut self, text: &str) {
        let trimmed = text.trim();

        if !trimmed.starts_with('/') || trimmed.len() < 2 {
            if trimmed == "/" {
                // Show all commands.
                self.suggestions = self
                    .commands
                    .iter()
                    .map(|c| Suggestion {
                        value: format!("/{}", c.name),
                        label: c.name.clone(),
                        description: c.description.clone(),
                    })
                    .collect();
                self.selected = 0;
                self.active = !self.suggestions.is_empty();
            } else {
                self.active = false;
                self.suggestions.clear();
            }
            return;
        }

        // Extract the command prefix after `/`.
        let prefix = &trimmed[1..];
        // Don't autocomplete if there's a space (command has args).
        if prefix.contains(' ') {
            self.active = false;
            self.suggestions.clear();
            return;
        }

        // Fuzzy filter commands.
        let filtered = fuzzy_filter(&self.commands, prefix, |c| &c.name);
        self.suggestions = filtered
            .into_iter()
            .map(|c| Suggestion {
                value: format!("/{}", c.name),
                label: c.name.clone(),
                description: c.description.clone(),
            })
            .collect();

        self.selected = 0;
        self.active = !self.suggestions.is_empty();
    }

    /// Whether the autocomplete dropdown is currently visible.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Dismiss the autocomplete.
    pub fn dismiss(&mut self) {
        self.active = false;
        self.suggestions.clear();
    }

    /// Take the result of the autocomplete interaction.
    pub fn take_result(&mut self) -> AutocompleteResult {
        std::mem::replace(&mut self.result, AutocompleteResult::Pending)
    }
}

impl Component for Autocomplete {
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
                theme::accent(&format!("/{}", s.label))
            } else {
                format!("/{}", s.label)
            };
            let desc = theme::dim(&s.description);
            let line = format!("{}{}  {}", prefix, name, desc);
            lines.push(truncate_to_width(&line, width, None));
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
        assert_eq!(ac.suggestions.len(), 4);
    }

    #[test]
    fn filters_on_prefix() {
        let mut ac = Autocomplete::new(test_commands(), 5);
        ac.update("/mo");
        assert!(ac.is_active());
        assert_eq!(ac.suggestions.len(), 1);
        assert_eq!(ac.suggestions[0].label, "model");
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
        let first = ac.suggestions[0].label.clone();
        ac.handle_input(&Key::Down);
        let second = ac.suggestions[ac.selected].label.clone();
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
}
