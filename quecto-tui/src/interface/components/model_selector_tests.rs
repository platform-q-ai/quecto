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
fn known_models_include_gpt_5_6_tiers_for_both_auth_modes() {
    let known_ids: Vec<String> = known_ids();
    for id in ["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"] {
        for auth in ["api", "oauth"] {
            let full = format!("openai-{auth}/{id}");
            assert!(
                known_ids.iter().any(|k| k == &full),
                "known models should include {full}: {:?}",
                known_ids
            );
        }
    }
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
    let clean = crate::interface::ansi::sanitize_control(dirty);
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
    // \x1b is a control character — sanitize_control strips it.
    // The remaining "[31m" is harmless text (not a valid escape sequence).
    let mut sel = ModelSelector::new(Some("evil\x1b[31mmodel"));
    let lines = sel.render(60);
    let _raw = lines.join("");
    // The injected \x1b should be stripped by sanitize_control.
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
