//! Effort selector overlay (#1067) — the provider-scoped reasoning-effort
//! vocabulary with fuzzy filtering, mirroring the model selector.
//!
//! Opened by `/effort` (no args). Lists ONLY the levels valid for the active
//! model's provider; the currently active level is marked. Selecting a level
//! sends a `set_effort` command to the agent.

use crate::components::autocomplete::Suggestion;
use crate::components::component::Component;
use crate::components::fuzzy::fuzzy_filter;
use crate::components::list_rows::{DescriptionMode, ListRow};
use crate::components::suggestion_list::SuggestionList;
use crate::components::theme;
use crate::components::utils::truncate_to_width;
use crate::shell::keys::Key;

/// Maximum query length to prevent unbounded growth.
const MAX_QUERY_LEN: usize = 16;

/// Result of the effort selector interaction — the shared list-interaction
/// result (`Selected` / `Dismissed` / `Pending`).
pub use crate::components::autocomplete::AutocompleteResult as EffortSelectorResult;

/// Scrollable effort selector with fuzzy search.
pub struct EffortSelector {
    /// The provider-scoped vocabulary (unfiltered).
    levels: Vec<String>,
    /// Shared filtered/selection state; each suggestion's `value` IS the level.
    list: SuggestionList,
    /// Fuzzy search query.
    query: String,
    /// The currently active level (marked with ●).
    current: Option<String>,
    /// Interaction result.
    result: EffortSelectorResult,
}

impl EffortSelector {
    /// Create a selector over the given provider-scoped vocabulary.
    /// `current` is the active level, marked in the list when present.
    pub fn new(levels: &[&str], current: Option<&str>) -> Self {
        let levels: Vec<String> = levels.iter().map(|l| (*l).to_string()).collect();
        let mut list = SuggestionList::new(8);
        list.set_suggestions(levels.iter().map(|l| to_suggestion(l)).collect());
        Self {
            levels,
            list,
            query: String::new(),
            current: current.map(str::to_string),
            result: EffortSelectorResult::Pending,
        }
    }

    /// Take the interaction result, resetting to Pending.
    pub fn take_result(&mut self) -> EffortSelectorResult {
        std::mem::replace(&mut self.result, EffortSelectorResult::Pending)
    }

    /// The levels currently visible (after filtering), in display order.
    /// Test seam — gated like its only consumers (the cfg'd test-harness
    /// probes) so a plain build never ships a zero-caller accessor.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn visible_levels(&self) -> Vec<String> {
        self.list
            .suggestions()
            .iter()
            .map(|s| s.value.clone())
            .collect()
    }

    fn update_filter(&mut self) {
        let suggestions: Vec<Suggestion> = if self.query.is_empty() {
            self.levels.iter().map(|l| to_suggestion(l)).collect()
        } else {
            fuzzy_filter(&self.levels, &self.query, |l| l.as_str())
                .into_iter()
                .map(|l| to_suggestion(l))
                .collect()
        };
        self.list.set_suggestions_clamping(suggestions);
    }
}

fn to_suggestion(level: &str) -> Suggestion {
    Suggestion {
        value: level.to_string(),
        description: String::new(),
    }
}

impl Component for EffortSelector {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        lines.push(truncate_to_width(
            &format!(
                "  {} {}",
                theme::bold("Select Effort"),
                theme::dim("(type to filter)")
            ),
            width,
            None,
        ));
        let search_line = if self.query.is_empty() {
            format!("  {}", theme::dim("Search: _"))
        } else {
            format!("  Search: {}{}", self.query, theme::dim("_"))
        };
        lines.push(truncate_to_width(&search_line, width, None));
        lines.push(String::new()); // spacer

        if self.list.is_empty() {
            lines.push(truncate_to_width(
                &format!("  {}", theme::dim("No matching levels")),
                width,
                None,
            ));
            return lines;
        }

        let current = self.current.as_deref();
        lines.extend(self.list.render_rows(
            width,
            "  ",
            DescriptionMode::AlignedCached { label_width: 0 },
            |s| ListRow {
                marker: if current == Some(s.value.as_str()) {
                    " ●"
                } else {
                    ""
                },
                ..ListRow::plain(s.value.clone())
            },
        ));
        lines
    }

    fn handle_input(&mut self, key: &Key) -> bool {
        match key {
            Key::Up => self.list.move_previous(),
            Key::Down => self.list.move_next(),
            Key::Enter => {
                // With no matches, Enter cancels.
                self.result = match self.list.selected_suggestion() {
                    Some(s) => EffortSelectorResult::Selected(s.value.clone()),
                    None => EffortSelectorResult::Dismissed,
                };
            }
            Key::Escape => self.result = EffortSelectorResult::Dismissed,
            Key::Backspace => {
                self.query.pop();
                self.update_filter();
            }
            Key::Char(c) => {
                if self.query.len() < MAX_QUERY_LEN {
                    self.query.push(*c);
                    self.update_filter();
                }
            }
            _ => return false,
        }
        true
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
#[path = "effort_selector_tests.rs"]
mod tests;
