//! Model selector overlay — scrollable list with fuzzy search.
//!
//! Opened by `/model` (no args) or Ctrl+L. Shows a list of available
//! models with fuzzy filtering. The currently active model is marked.
//! Selecting a model sends a `set_model` command to the agent.

use crate::interface::component::Component;
use crate::interface::components::list_navigator::ListNavigator;
use crate::interface::components::sanitize::strip_terminal_control;
use crate::interface::fuzzy::fuzzy_filter;
use crate::interface::keys::Key;
use crate::interface::theme;
use crate::interface::utils::{truncate_to_width, visible_width};

/// Well-known model identifiers, used as fallback when the caller
/// doesn't supply a model list.
const KNOWN_MODELS: &[(&str, &str)] = &[
    ("anthropic-api/claude-fable-5", "Anthropic API"),
    ("anthropic-oauth/claude-fable-5", "Anthropic OAuth"),
    ("anthropic-api/claude-opus-4-8", "Anthropic API"),
    ("anthropic-oauth/claude-opus-4-8", "Anthropic OAuth"),
    ("anthropic-api/claude-opus-4-7", "Anthropic API"),
    ("anthropic-oauth/claude-opus-4-7", "Anthropic OAuth"),
    ("anthropic-api/claude-opus-4-6", "Anthropic API"),
    ("anthropic-oauth/claude-opus-4-6", "Anthropic OAuth"),
    ("anthropic-api/claude-opus-4-5", "Anthropic API"),
    ("anthropic-oauth/claude-opus-4-5", "Anthropic OAuth"),
    ("anthropic-api/claude-sonnet-4-6", "Anthropic API"),
    ("anthropic-oauth/claude-sonnet-4-6", "Anthropic OAuth"),
    ("anthropic-api/claude-sonnet-4-5", "Anthropic API"),
    ("anthropic-oauth/claude-sonnet-4-5", "Anthropic OAuth"),
    ("openai-api/gpt-5.5", "OpenAI API"),
    ("openai-oauth/gpt-5.5", "OpenAI OAuth"),
    ("openai-api/gpt-5.5-mini", "OpenAI API"),
    ("openai-oauth/gpt-5.5-mini", "OpenAI OAuth"),
    ("openai-api/gpt-5.5-nano", "OpenAI API"),
    ("openai-oauth/gpt-5.5-nano", "OpenAI OAuth"),
    ("openai-api/gpt-5.3-codex", "OpenAI API"),
    ("openai-oauth/gpt-5.3-codex", "OpenAI OAuth"),
    ("openai-api/gpt-5.3-codex-spark", "OpenAI API"),
    ("openai-oauth/gpt-5.3-codex-spark", "OpenAI OAuth"),
    ("openai-api/gpt-5.2-codex", "OpenAI API"),
    ("openai-oauth/gpt-5.2-codex", "OpenAI OAuth"),
    ("fireworks/accounts/fireworks/models/glm-5p2", "Fireworks"),
    (
        "fireworks/accounts/fireworks/models/kimi-k2p7-code",
        "Fireworks",
    ),
];

/// Maximum query length to prevent unbounded growth.
const MAX_QUERY_LEN: usize = 64;

/// Sanitize a model name by stripping control characters.
///
/// Prevents terminal escape injection via agent-sourced model names.
fn sanitize_model_name(name: &str) -> String {
    strip_terminal_control(name)
}

/// A model entry in the selector.
#[derive(Debug, Clone)]
pub struct ModelEntry {
    pub id: String,
    pub provider: String,
    /// Human-readable auth label shown in the selector (e.g. "oauth" or "api").
    pub auth: Option<String>,
    pub is_current: bool,
}

/// Result of the model selector interaction.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelSelectorResult {
    /// User selected a model.
    Selected(String),
    /// User cancelled (Escape).
    Cancelled,
    /// No action yet.
    Pending,
}

/// Scrollable model selector with fuzzy search.
pub struct ModelSelector {
    /// All available models (unfiltered).
    all_models: Vec<ModelEntry>,
    /// Filtered models based on query.
    filtered: Vec<usize>,
    /// Current selection index into `filtered`.
    navigator: ListNavigator,
    /// Fuzzy search query.
    query: String,
    /// Maximum visible items.
    max_visible: usize,
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
        let models: Vec<ModelEntry> = KNOWN_MODELS
            .iter()
            .map(|(id, provider)| ModelEntry {
                id: id.to_string(),
                provider: provider.to_string(),
                auth: None,
                is_current: false,
            })
            .collect();
        Self::with_models(models, current_model)
    }

    /// Create a model selector with a caller-supplied model list.
    ///
    /// This decouples the component from the hardcoded known models,
    /// allowing future integration with dynamic model lists from the agent.
    pub fn with_models(mut models: Vec<ModelEntry>, current_model: Option<&str>) -> Self {
        // Sanitize and mark the current model.
        if let Some(current) = current_model {
            let safe_current = sanitize_model_name(current);
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

        let filtered: Vec<usize> = (0..models.len()).collect();
        let cached_width = compute_max_label_width(&models, &filtered);

        Self {
            all_models: models,
            filtered,
            navigator: ListNavigator::new(),
            query: String::new(),
            max_visible: 12,
            result: ModelSelectorResult::Pending,
            cached_max_label_width: cached_width,
        }
    }

    /// Take the interaction result, resetting to Pending.
    pub fn take_result(&mut self) -> ModelSelectorResult {
        std::mem::replace(&mut self.result, ModelSelectorResult::Pending)
    }

    /// Update the filtered list based on the current query.
    fn update_filter(&mut self) {
        if self.query.is_empty() {
            self.filtered = (0..self.all_models.len()).collect();
        } else {
            // Build indexed pairs without cloning model IDs.
            // `all_models` and `query` are separate fields, so no borrow conflict.
            let indexed: Vec<(usize, &str)> = self
                .all_models
                .iter()
                .enumerate()
                .map(|(i, m)| (i, m.id.as_str()))
                .collect();
            let matching = fuzzy_filter(&indexed, &self.query, |item| item.1);
            self.filtered = matching.into_iter().map(|item| item.0).collect();
        }
        self.navigator.clamp(self.filtered.len());
        // Recache label width.
        self.cached_max_label_width = compute_max_label_width(&self.all_models, &self.filtered);
    }

    /// Get the currently selected model entry, if any.
    pub fn selected_model(&self) -> Option<&ModelEntry> {
        self.filtered
            .get(self.navigator.selected())
            .map(|&idx| &self.all_models[idx])
    }

    /// Get the number of visible (filtered) models.
    pub fn visible_count(&self) -> usize {
        self.filtered.len()
    }

    /// The shared [`SuggestionList`] state backing the selector's rows (#997).
    ///
    /// Contract: each suggestion's `value` IS the model id (never an index or
    /// a hollow empty value), so `SuggestionList::set_suggestions` change
    /// detection keeps working. RED scaffold — wired when the selector
    /// migrates off its parallel `filtered`/index machinery.
    #[cfg(test)]
    pub(crate) fn shared_list(
        &self,
    ) -> &crate::interface::components::suggestion_list::SuggestionList {
        unimplemented!("issue #997: ModelSelector does not yet reuse SuggestionList")
    }
}

/// Compute the max label width across filtered entries.
fn compute_max_label_width(models: &[ModelEntry], filtered: &[usize]) -> usize {
    filtered
        .iter()
        .map(|&idx| visible_width(&models[idx].id))
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
        if self.filtered.is_empty() {
            lines.push(truncate_to_width(
                &format!("  {}", theme::dim("No matching models")),
                width,
                None,
            ));
            return lines;
        }

        // Calculate visible window.
        let total = self.filtered.len();
        let range = self.navigator.visible_range(total, self.max_visible);
        let start = range.start;
        let end = range.end;

        // Use cached label width for alignment.
        let max_label_width = self.cached_max_label_width;

        for i in start..end {
            let idx = self.filtered[i];
            let model = &self.all_models[idx];
            let is_sel = i == self.navigator.selected();

            let prefix = if is_sel { "→ " } else { "  " };
            let current_marker = if model.is_current { " ●" } else { "" };

            let label = if is_sel {
                theme::accent(&model.id)
            } else {
                model.id.clone()
            };

            let label_vis = visible_width(&model.id);
            let gap = max_label_width.saturating_sub(label_vis) + 2;
            let spacing = " ".repeat(gap);
            let provider_label = match model.auth.as_deref() {
                Some(auth) if !auth.is_empty() => format!("{} [{}]", model.provider, auth),
                _ => model.provider.clone(),
            };
            let provider_str = theme::dim(&provider_label);

            let line = format!(
                "  {}{}{}{}{}",
                prefix, label, current_marker, spacing, provider_str
            );
            lines.push(truncate_to_width(&line, width, None));
        }

        // Scroll indicator.
        if start > 0 || end < total {
            lines.push(truncate_to_width(
                &format!(
                    "  {}",
                    theme::dim(&format!("({}/{})", self.navigator.selected() + 1, total))
                ),
                width,
                None,
            ));
        }

        lines
    }

    fn handle_input(&mut self, key: &Key) -> bool {
        match key {
            Key::Up => {
                self.navigator.move_previous(self.filtered.len());
                true
            }
            Key::Down => {
                self.navigator.move_next(self.filtered.len());
                true
            }
            Key::Enter => {
                if let Some(model) = self.selected_model() {
                    self.result = ModelSelectorResult::Selected(model.id.clone());
                } else {
                    // No matches — treat Enter as cancel.
                    self.result = ModelSelectorResult::Cancelled;
                }
                true
            }
            Key::Escape => {
                self.result = ModelSelectorResult::Cancelled;
                true
            }
            Key::Backspace => {
                self.query.pop();
                self.update_filter();
                true
            }
            Key::Char(c) => {
                if self.query.len() < MAX_QUERY_LEN {
                    self.query.push(*c);
                    self.update_filter();
                }
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
        let known_ids: Vec<&str> = KNOWN_MODELS.iter().map(|(id, _)| *id).collect();
        assert!(
            known_ids.contains(&"anthropic-api/claude-fable-5"),
            "known models should include Claude Fable 5: {:?}",
            known_ids
        );
        assert!(
            known_ids.contains(&"anthropic-api/claude-opus-4-8"),
            "known models should include Opus 4.8: {:?}",
            known_ids
        );
        assert!(
            known_ids.contains(&"anthropic-api/claude-opus-4-7"),
            "known models should include Opus 4.7: {:?}",
            known_ids
        );
    }

    #[test]
    fn known_models_include_fireworks_serverless_models() {
        let known_ids: Vec<&str> = KNOWN_MODELS.iter().map(|(id, _)| *id).collect();
        assert!(
            known_ids.contains(&"fireworks/accounts/fireworks/models/glm-5p2"),
            "known models should include Fireworks GLM 5.2: {:?}",
            known_ids
        );
        assert!(
            known_ids.contains(&"fireworks/accounts/fireworks/models/kimi-k2p7-code"),
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
        assert_eq!(selected.id, KNOWN_MODELS[1].0, "should select second model");
    }

    #[test]
    fn navigate_down_wraps() {
        let mut sel = ModelSelector::new(None);
        let count = sel.visible_count();
        for _ in 0..count {
            sel.handle_input(&Key::Down);
        }
        let selected = sel.selected_model().unwrap();
        assert_eq!(selected.id, KNOWN_MODELS[0].0, "should wrap to first model");
    }

    #[test]
    fn navigate_up_wraps() {
        let mut sel = ModelSelector::new(None);
        sel.handle_input(&Key::Up);
        let selected = sel.selected_model().unwrap();
        let last_idx = KNOWN_MODELS.len() - 1;
        assert_eq!(
            selected.id, KNOWN_MODELS[last_idx].0,
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
            ModelSelectorResult::Selected(KNOWN_MODELS[0].0.to_string())
        );
    }

    #[test]
    fn escape_cancels() {
        let mut sel = ModelSelector::new(None);
        sel.handle_input(&Key::Escape);
        assert_eq!(sel.take_result(), ModelSelectorResult::Cancelled);
    }

    #[test]
    fn fuzzy_filter_narrows_list() {
        let mut sel = ModelSelector::new(None);
        sel.handle_input(&Key::Char('s'));
        sel.handle_input(&Key::Char('o'));
        sel.handle_input(&Key::Char('n'));
        assert!(
            sel.visible_count() < KNOWN_MODELS.len(),
            "filter should reduce visible count: {} vs {}",
            sel.visible_count(),
            KNOWN_MODELS.len()
        );
        let visible_ids: Vec<&str> = sel
            .filtered
            .iter()
            .map(|&idx| sel.all_models[idx].id.as_str())
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
        assert_eq!(sel.visible_count(), KNOWN_MODELS.len());
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
        assert_eq!(list.len(), KNOWN_MODELS.len());
        assert_eq!(
            list.suggestions()[0].value,
            KNOWN_MODELS[0].0,
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

    // ── Review fix tests ──────────────────────────────────────────────

    #[test]
    fn sanitize_strips_control_chars() {
        let dirty = "model\x1b[31m-evil\x07name";
        let clean = sanitize_model_name(dirty);
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
        assert_eq!(sel.take_result(), ModelSelectorResult::Cancelled);
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
        // \x1b is a control character — sanitize_model_name strips it.
        // The remaining "[31m" is harmless text (not a valid escape sequence).
        let mut sel = ModelSelector::new(Some("evil\x1b[31mmodel"));
        let lines = sel.render(60);
        let _raw = lines.join("");
        // The injected \x1b should be stripped by sanitize_model_name.
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
