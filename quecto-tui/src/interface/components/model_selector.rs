//! Model selector overlay — scrollable list with fuzzy search.
//!
//! Opened by `/model` (no args) or Ctrl+L. Shows a list of available
//! models with fuzzy filtering. The currently active model is marked.
//! Selecting a model sends a `set_model` command to the agent.

use crate::interface::component::Component;
use crate::interface::components::autocomplete::Suggestion;
use crate::interface::components::list_rows::{DescriptionMode, ListRow};
use crate::interface::components::sanitize::strip_terminal_control;
use crate::interface::components::suggestion_list::SuggestionList;
use crate::interface::fuzzy::fuzzy_filter;
use crate::interface::keys::Key;
use crate::interface::theme;
use crate::interface::utils::{truncate_to_width, visible_width};

/// Well-known fallback models, used when the caller doesn't supply a model
/// list: every Anthropic/OpenAI model is offered through both its `api` and
/// `oauth` provider; the Fireworks serverless ids are single-provider.
fn known_models() -> Vec<ModelEntry> {
    const ANTHROPIC: &[&str] = &[
        "claude-fable-5",
        "claude-opus-4-8",
        "claude-opus-4-7",
        "claude-opus-4-6",
        "claude-opus-4-5",
        "claude-sonnet-4-6",
        "claude-sonnet-4-5",
    ];
    const OPENAI: &[&str] = &[
        "gpt-5.5",
        "gpt-5.5-mini",
        "gpt-5.5-nano",
        "gpt-5.3-codex",
        "gpt-5.3-codex-spark",
        "gpt-5.2-codex",
    ];
    let mut pairs: Vec<(String, String)> = Vec::new();
    for (vendor, brand, ids) in [
        ("anthropic", "Anthropic", ANTHROPIC),
        ("openai", "OpenAI", OPENAI),
    ] {
        for id in ids {
            for (auth, label) in [("api", "API"), ("oauth", "OAuth")] {
                pairs.push((format!("{vendor}-{auth}/{id}"), format!("{brand} {label}")));
            }
        }
    }
    for id in ["glm-5p2", "kimi-k2p7-code"] {
        pairs.push((
            format!("fireworks/accounts/fireworks/models/{id}"),
            "Fireworks".into(),
        ));
    }
    pairs
        .into_iter()
        .map(|(id, provider)| ModelEntry {
            id,
            provider,
            auth: None,
            is_current: false,
        })
        .collect()
}

/// Maximum query length to prevent unbounded growth.
const MAX_QUERY_LEN: usize = 64;

/// A model entry in the selector.
#[derive(Debug, Clone)]
pub struct ModelEntry {
    pub id: String,
    pub provider: String,
    /// Human-readable auth label shown in the selector (e.g. "oauth" or "api").
    pub auth: Option<String>,
    pub is_current: bool,
}

/// Result of the model selector interaction — the shared list-interaction
/// result (`Selected` / `Dismissed` / `Pending`), re-exported under this
/// surface's historical name.
pub use crate::interface::components::autocomplete::AutocompleteResult as ModelSelectorResult;

/// Scrollable model selector with fuzzy search.
pub struct ModelSelector {
    /// All available models (unfiltered).
    all_models: Vec<ModelEntry>,
    /// Shared filtered/selection state (#997): each suggestion's `value` IS
    /// the model id (never an index or hollow value), so
    /// `SuggestionList::set_suggestions` change detection stays meaningful.
    list: SuggestionList,
    /// Fuzzy search query.
    query: String,
    /// Interaction result.
    result: ModelSelectorResult,
    /// Cached max label width (recalculated only when filter changes).
    cached_max_label_width: usize,
}

impl ModelSelector {
    /// Create a new model selector with default known models.
    ///
    /// `current_model` is sanitized and marked with ● in the list.
    /// If it's not in the known list, it's added at the top.
    pub fn new(current_model: Option<&str>) -> Self {
        Self::with_models(known_models(), current_model)
    }

    /// Create a model selector with a caller-supplied model list.
    ///
    /// This decouples the component from the hardcoded known models,
    /// allowing future integration with dynamic model lists from the agent.
    pub fn with_models(mut models: Vec<ModelEntry>, current_model: Option<&str>) -> Self {
        // Sanitize and mark the current model.
        if let Some(current) = current_model {
            // Strip control characters — prevents terminal escape injection
            // via agent-sourced model names.
            let safe_current = strip_terminal_control(current);
            let mut found = false;
            for m in &mut models {
                if m.id == safe_current {
                    m.is_current = true;
                    found = true;
                }
            }
            // If the current model isn't in the list, add it at the top.
            if !found && !safe_current.is_empty() {
                models.insert(
                    0,
                    ModelEntry {
                        id: safe_current,
                        provider: "Custom".to_string(),
                        auth: None,
                        is_current: true,
                    },
                );
            }
        }

        let suggestions: Vec<Suggestion> = models.iter().map(to_suggestion).collect();
        let cached_width = compute_max_label_width(&suggestions);
        let mut list = SuggestionList::new(12);
        list.set_suggestions(suggestions);

        Self {
            all_models: models,
            list,
            query: String::new(),
            result: ModelSelectorResult::Pending,
            cached_max_label_width: cached_width,
        }
    }

    /// Take the interaction result, resetting to Pending.
    pub fn take_result(&mut self) -> ModelSelectorResult {
        std::mem::replace(&mut self.result, ModelSelectorResult::Pending)
    }

    /// Update the filtered list based on the current query. The selection is
    /// CLAMPED into the new range (historical semantics), not reset to row 0.
    fn update_filter(&mut self) {
        let suggestions: Vec<Suggestion> = if self.query.is_empty() {
            self.all_models.iter().map(to_suggestion).collect()
        } else {
            fuzzy_filter(&self.all_models, &self.query, |m| &m.id)
                .into_iter()
                .map(to_suggestion)
                .collect()
        };
        // Recache label width — only when the filter changes, never per frame
        // over the full filtered list (#757).
        self.cached_max_label_width = compute_max_label_width(&suggestions);
        self.list.set_suggestions_clamping(suggestions);
    }

    /// Get the currently selected model entry, if any.
    pub fn selected_model(&self) -> Option<&ModelEntry> {
        let id = &self.list.selected_suggestion()?.value;
        self.entry_by_id(id)
    }

    /// Get the number of visible (filtered) models.
    pub fn visible_count(&self) -> usize {
        self.list.len()
    }

    /// Look up the [`ModelEntry`] backing a suggestion by its model id.
    fn entry_by_id(&self, id: &str) -> Option<&ModelEntry> {
        self.all_models.iter().find(|m| m.id == id)
    }
}

/// Build the suggestion for a model: `value` is the model id itself (also the
/// display label); the description is the dim provider column (with the auth
/// suffix, if any).
fn to_suggestion(m: &ModelEntry) -> Suggestion {
    let description = match m.auth.as_deref() {
        Some(auth) if !auth.is_empty() => format!("{} [{}]", m.provider, auth),
        _ => m.provider.clone(),
    };
    Suggestion {
        value: m.id.clone(),
        description,
    }
}

/// Compute the max label width across filtered entries.
fn compute_max_label_width(suggestions: &[Suggestion]) -> usize {
    suggestions
        .iter()
        .map(|s| visible_width(&s.value))
        .max()
        .unwrap_or(10)
        .min(40)
}

impl Component for ModelSelector {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();

        // Title.
        lines.push(truncate_to_width(
            &format!(
                "  {} {}",
                theme::bold("Select Model"),
                theme::dim("(type to filter)")
            ),
            width,
            None,
        ));

        // Search input.
        let search_line = if self.query.is_empty() {
            format!("  {}", theme::dim("Search: _"))
        } else {
            format!("  Search: {}{}", self.query, theme::dim("_"))
        };
        lines.push(truncate_to_width(&search_line, width, None));
        lines.push(String::new()); // spacer

        // Filtered list.
        if self.list.is_empty() {
            lines.push(truncate_to_width(
                &format!("  {}", theme::dim("No matching models")),
                width,
                None,
            ));
            return lines;
        }

        // Shared row renderer (#997): 2-space indent, provider column on the
        // cached width (#757), ` ●` marker outside the alignment column.
        let mode = DescriptionMode::AlignedCached {
            label_width: self.cached_max_label_width,
        };
        // Resolve the current model id ONCE per frame — never a per-row
        // linear scan of `all_models` (#757 hot-path parity with the
        // pre-#997 index-based renderer).
        let current_id: Option<&str> = self
            .all_models
            .iter()
            .find(|m| m.is_current)
            .map(|m| m.id.as_str());
        lines.extend(self.list.render_rows(width, "  ", mode, |s| {
            let is_current = current_id == Some(s.value.as_str());
            ListRow {
                description: Some(s.description.clone()),
                marker: if is_current { " ●" } else { "" },
                ..ListRow::plain(s.value.clone())
            }
        }));
        lines
    }

    fn handle_input(&mut self, key: &Key) -> bool {
        match key {
            Key::Up => self.list.move_previous(),
            Key::Down => self.list.move_next(),
            Key::Enter => {
                // With no matches, Enter cancels.
                self.result = match self.selected_model() {
                    Some(model) => ModelSelectorResult::Selected(model.id.clone()),
                    None => ModelSelectorResult::Dismissed,
                };
            }
            Key::Escape => self.result = ModelSelectorResult::Dismissed,
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
mod tests {
    use super::*;

    impl ModelSelector {
        /// The shared [`SuggestionList`] state backing the selector's rows
        /// (#997). Contract: each suggestion's `value` IS the model id (never
        /// an index or a hollow empty value), so
        /// `SuggestionList::set_suggestions` change detection keeps working.
        pub(crate) fn shared_list(&self) -> &SuggestionList {
            &self.list
        }
    }

    /// The default model ids, in selector order (test stand-in for the old
    /// hardcoded table).
    fn known_ids() -> Vec<String> {
        known_models().into_iter().map(|m| m.id).collect()
    }

    fn strip_ansi(s: &str) -> String {
        let mut result = String::new();
        let mut in_escape = false;
        for ch in s.chars() {
            if in_escape {
                if ch.is_ascii_alphabetic() || ch == '~' {
                    in_escape = false;
                }
            } else if ch == '\x1b' {
                in_escape = true;
            } else {
                result.push(ch);
            }
        }
        result
    }

    #[test]
    fn renders_model_list() {
        let mut sel = ModelSelector::new(None);
        let lines = sel.render(60);
        let plain: String = lines
            .iter()
            .map(|l| strip_ansi(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            plain.contains("claude-sonnet-4"),
            "should contain a model: {}",
            plain
        );
    }

    #[test]
    fn known_models_include_latest_anthropic_models() {
        let known_ids: Vec<String> = known_ids();
        assert!(
            known_ids
                .iter()
                .any(|id| id == "anthropic-api/claude-fable-5"),
            "known models should include Claude Fable 5: {:?}",
            known_ids
        );
        assert!(
            known_ids
                .iter()
                .any(|id| id == "anthropic-api/claude-opus-4-8"),
            "known models should include Opus 4.8: {:?}",
            known_ids
        );
        assert!(
            known_ids
                .iter()
                .any(|id| id == "anthropic-api/claude-opus-4-7"),
            "known models should include Opus 4.7: {:?}",
            known_ids
        );
    }

    #[test]
    fn known_models_include_fireworks_serverless_models() {
        let known_ids: Vec<String> = known_ids();
        assert!(
            known_ids
                .iter()
                .any(|id| id == "fireworks/accounts/fireworks/models/glm-5p2"),
            "known models should include Fireworks GLM 5.2: {:?}",
            known_ids
        );
        assert!(
            known_ids
                .iter()
                .any(|id| id == "fireworks/accounts/fireworks/models/kimi-k2p7-code"),
            "known models should include Fireworks Kimi K2.7 Code: {:?}",
            known_ids
        );
    }

    #[test]
    fn shows_selection_indicator() {
        let mut sel = ModelSelector::new(None);
        let lines = sel.render(60);
        let joined: String = lines
            .iter()
            .map(|l| strip_ansi(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            joined.contains('→'),
            "should show selection indicator: {}",
            joined
        );
    }

    #[test]
    fn navigate_down_selects_next() {
        let mut sel = ModelSelector::new(None);
        sel.handle_input(&Key::Down);
        let selected = sel.selected_model().unwrap();
        assert_eq!(selected.id, known_ids()[1], "should select second model");
    }

    #[test]
    fn navigate_down_wraps() {
        let mut sel = ModelSelector::new(None);
        let count = sel.visible_count();
        for _ in 0..count {
            sel.handle_input(&Key::Down);
        }
        let selected = sel.selected_model().unwrap();
        assert_eq!(selected.id, known_ids()[0], "should wrap to first model");
    }

    #[test]
    fn navigate_up_wraps() {
        let mut sel = ModelSelector::new(None);
        sel.handle_input(&Key::Up);
        let selected = sel.selected_model().unwrap();
        let last_idx = known_ids().len() - 1;
        assert_eq!(
            selected.id,
            known_ids()[last_idx],
            "should wrap to last model"
        );
    }

    #[test]
    fn enter_selects_model() {
        let mut sel = ModelSelector::new(None);
        sel.handle_input(&Key::Enter);
        let result = sel.take_result();
        assert_eq!(
            result,
            ModelSelectorResult::Selected(known_ids()[0].clone())
        );
    }

    #[test]
    fn escape_cancels() {
        let mut sel = ModelSelector::new(None);
        sel.handle_input(&Key::Escape);
        assert_eq!(sel.take_result(), ModelSelectorResult::Dismissed);
    }

    #[test]
    fn fuzzy_filter_narrows_list() {
        let mut sel = ModelSelector::new(None);
        sel.handle_input(&Key::Char('s'));
        sel.handle_input(&Key::Char('o'));
        sel.handle_input(&Key::Char('n'));
        assert!(
            sel.visible_count() < known_ids().len(),
            "filter should reduce visible count: {} vs {}",
            sel.visible_count(),
            known_ids().len()
        );
        let visible_ids: Vec<&str> = sel
            .list
            .suggestions()
            .iter()
            .map(|s| s.value.as_str())
            .collect();
        assert!(
            visible_ids.iter().any(|id| id.contains("sonnet")),
            "should contain sonnet: {:?}",
            visible_ids
        );
    }

    #[test]
    fn empty_query_shows_all() {
        let mut sel = ModelSelector::new(None);
        sel.handle_input(&Key::Char('x'));
        sel.handle_input(&Key::Backspace);
        assert_eq!(sel.visible_count(), known_ids().len());
    }

    #[test]
    fn no_match_shows_empty_state() {
        let mut sel = ModelSelector::new(None);
        for c in "zzzznonexistent".chars() {
            sel.handle_input(&Key::Char(c));
        }
        assert_eq!(sel.visible_count(), 0);
        let lines = sel.render(60);
        let plain: String = lines
            .iter()
            .map(|l| strip_ansi(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            plain.contains("No matching models"),
            "should show empty state: {}",
            plain
        );
    }

    #[test]
    fn current_model_marked() {
        let mut sel = ModelSelector::new(Some("claude-sonnet-4-6"));
        let lines = sel.render(60);
        let plain: String = lines
            .iter()
            .map(|l| strip_ansi(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            plain.contains('●'),
            "should show current model marker: {}",
            plain
        );
    }

    #[test]
    fn custom_model_added_when_not_in_known() {
        let mut sel = ModelSelector::new(Some("my-custom-model"));
        let lines = sel.render(60);
        let plain: String = lines
            .iter()
            .map(|l| strip_ansi(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            plain.contains("my-custom-model"),
            "should show custom model: {}",
            plain
        );
        assert!(
            plain.contains('●'),
            "custom model should be marked as current: {}",
            plain
        );
    }

    #[test]
    fn respects_width() {
        let mut sel = ModelSelector::new(Some("claude-sonnet-4-6"));
        let lines = sel.render(40);
        for line in &lines {
            assert!(
                visible_width(line) <= 40,
                "line exceeds width: '{}' (width={})",
                strip_ansi(line),
                visible_width(line)
            );
        }
    }

    // ── #997 shared-state contract (RED until the SuggestionList migration) ──

    #[test]
    fn shared_list_backs_rows_with_model_id_values() {
        let sel = ModelSelector::new(None);
        let list = sel.shared_list();
        assert_eq!(list.len(), known_ids().len());
        assert_eq!(
            list.suggestions()[0].value,
            known_ids()[0],
            "Suggestion.value must carry the model id itself, not an index"
        );
    }

    #[test]
    fn shared_list_selection_clamps_when_filter_narrows() {
        let mut sel = ModelSelector::new(None);
        for _ in 0..5 {
            sel.handle_input(&Key::Down);
        }
        for c in "fireworks".chars() {
            sel.handle_input(&Key::Char(c));
        }
        assert_eq!(
            sel.shared_list().selected(),
            1,
            "shared state must preserve the clamp-on-filter-change semantics"
        );
    }

    #[test]
    fn shared_list_suggestions_track_filter_changes() {
        // Guards the rejected-attempt bug: re-setting suggestions on a filter
        // change must actually replace the shared list's values (which only
        // works because `value` is the model id, not a hollow placeholder).
        let mut sel = ModelSelector::new(None);
        for c in "fireworks".chars() {
            sel.handle_input(&Key::Char(c));
        }
        let values: Vec<&str> = sel
            .shared_list()
            .suggestions()
            .iter()
            .map(|s| s.value.as_str())
            .collect();
        assert_eq!(values.len(), 2, "two fireworks models match");
        assert!(
            values.iter().all(|v| v.starts_with("fireworks/")),
            "{values:?}"
        );
        for _ in 0.."fireworks".len() {
            sel.handle_input(&Key::Backspace);
        }
        assert_eq!(
            sel.shared_list().len(),
            known_ids().len(),
            "clearing the filter restores the full list"
        );
        assert_eq!(sel.shared_list().suggestions()[0].value, known_ids()[0]);
    }

    // ── Review fix tests ──────────────────────────────────────────────

    #[test]
    fn sanitize_strips_control_chars() {
        let dirty = "model\x1b[31m-evil\x07name";
        let clean = strip_terminal_control(dirty);
        assert!(!clean.contains('\x1b'));
        assert!(!clean.contains('\x07'));
        assert!(clean.contains("model"));
        assert!(clean.contains("name"));
    }

    #[test]
    fn query_capped_at_max_length() {
        let mut sel = ModelSelector::new(None);
        for _ in 0..100 {
            sel.handle_input(&Key::Char('x'));
        }
        assert_eq!(sel.query.len(), MAX_QUERY_LEN);
    }

    #[test]
    fn enter_on_empty_filtered_cancels() {
        let mut sel = ModelSelector::new(None);
        for c in "zzzznonexistent".chars() {
            sel.handle_input(&Key::Char(c));
        }
        assert_eq!(sel.visible_count(), 0);
        sel.handle_input(&Key::Enter);
        assert_eq!(sel.take_result(), ModelSelectorResult::Dismissed);
    }

    #[test]
    fn with_models_accepts_custom_list() {
        let models = vec![
            ModelEntry {
                id: "model-a".to_string(),
                provider: "ProviderA".to_string(),
                auth: None,
                is_current: false,
            },
            ModelEntry {
                id: "model-b".to_string(),
                provider: "ProviderB".to_string(),
                auth: None,
                is_current: false,
            },
        ];
        let mut sel = ModelSelector::with_models(models, Some("model-a"));
        let lines = sel.render(60);
        let plain: String = lines
            .iter()
            .map(|l| strip_ansi(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(plain.contains("model-a"));
        assert!(plain.contains("model-b"));
        assert!(plain.contains('●')); // model-a is current
    }

    #[test]
    fn custom_model_with_control_chars_sanitized() {
        // \x1b is a control character — strip_terminal_control strips it.
        // The remaining "[31m" is harmless text (not a valid escape sequence).
        let mut sel = ModelSelector::new(Some("evil\x1b[31mmodel"));
        let lines = sel.render(60);
        let _raw = lines.join("");
        // The injected \x1b should be stripped by strip_terminal_control.
        // Count \x1b occurrences that are NOT from theme styling.
        // Verify the model ID stored is sanitized.
        let custom_entry = sel
            .all_models
            .iter()
            .find(|m| m.is_current)
            .expect("should have current model");
        assert!(
            !custom_entry.id.contains('\x1b'),
            "model id should not contain escape: {:?}",
            custom_entry.id
        );
        assert!(
            custom_entry.id.contains("evil"),
            "should preserve text: {}",
            custom_entry.id
        );
        assert!(
            custom_entry.id.contains("model"),
            "should preserve text: {}",
            custom_entry.id
        );
    }
}
