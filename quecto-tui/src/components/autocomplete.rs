//! Autocomplete dropdown — renders below the editor when typing `/`.
//!
//! Provides fuzzy-matched suggestions for slash commands and model names.

use crate::components::component::Component;
use crate::components::fuzzy::fuzzy_filter;
use crate::components::list_rows::{DescriptionMode, ListRow};
use crate::components::suggestion_list::SuggestionList;
use crate::shell::keys::Key;

/// A slash command definition.
#[derive(Debug, Clone)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
}

/// Autocomplete suggestion. `value` doubles as the display label (the
/// surfaces add their own `/`/`@` sigil or show it verbatim).
#[derive(Debug, Clone)]
pub struct Suggestion {
    pub value: String,
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
                let new = self.commands.iter().map(to_suggestion).collect();
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
        let new = filtered.into_iter().map(to_suggestion).collect();
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

    /// The 0-based index of the currently highlighted suggestion
    /// (tests/harness inspection accessor).
    pub fn selected_index(&self) -> usize {
        self.list.selected()
    }

    /// The `value` of the highlighted suggestion (e.g. `"/quit"`), if any.
    pub fn selected_value(&self) -> Option<String> {
        self.list.selected_suggestion().map(|s| s.value.clone())
    }

    /// Suggestions currently held. A Tab/Enter accept `close()`s (hides) the
    /// list but RETAINS its suggestions — distinct from `dismiss()`/`clear()`.
    pub fn suggestion_count(&self) -> usize {
        self.list.len()
    }
}

/// Build the dropdown suggestion for a slash command (`value` carries the
/// leading `/`, ready to submit — it IS the display label).
fn to_suggestion(c: &SlashCommand) -> Suggestion {
    Suggestion {
        value: format!("/{}", c.name),
        description: c.description.clone(),
    }
}

impl Component for Autocomplete {
    fn render(&mut self, width: usize) -> Vec<String> {
        if !self.list.is_active() || self.list.is_empty() {
            return vec![];
        }

        // Shared row renderer (#997): fixed two-space description gap
        // (`label_width: 0`); windowing + overflow indicator live in the helper.
        let mode = DescriptionMode::AlignedCached { label_width: 0 };
        self.list.render_rows(width, "", mode, |s| ListRow {
            description: Some(s.description.clone()),
            ..ListRow::plain(s.value.clone())
        })
    }

    fn handle_input(&mut self, key: &Key) -> bool {
        self.list.handle_key(key, true, &mut self.result)
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
#[path = "autocomplete_tests.rs"]
mod tests;
