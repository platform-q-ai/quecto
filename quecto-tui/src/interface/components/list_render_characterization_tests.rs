//! Pixel characterization of the four list/overlay renderers (#997).
//!
//! These tests lock the EXACT visible output (ANSI-stripped text, plus the
//! presence of accent/dim styling where it is semantically load-bearing) of
//! the select list, slash-command autocomplete, `@files` autocomplete and the
//! model selector, BEFORE they migrate onto the shared row renderer in
//! `list_rows`. They must pass unchanged before, during and after the #997
//! refactor: any diff here is a visual regression, not a refactor.

use std::time::{Duration, Instant};

use crate::interface::component::Component;
use crate::interface::components::autocomplete::{Autocomplete, SlashCommand};
use crate::interface::components::files_autocomplete::FilesAutocomplete;
use crate::interface::components::model_selector::{ModelEntry, ModelSelector};
use crate::interface::components::select_list::{SelectItem, SelectList};
use crate::interface::keys::Key;
use crate::interface::utils::visible_width;

const ACCENT: &str = "\x1b[36m";
const DIM: &str = "\x1b[2m";

fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut esc = false;
    for ch in s.chars() {
        if esc {
            if ch.is_ascii_alphabetic() || ch == '~' {
                esc = false;
            }
        } else if ch == '\x1b' {
            esc = true;
        } else {
            out.push(ch);
        }
    }
    out
}

fn stripped(lines: &[String]) -> Vec<String> {
    lines.iter().map(|l| strip_ansi(l)).collect()
}

// ── Select list ──────────────────────────────────────────────────────────────

fn select_items() -> Vec<SelectItem> {
    [
        ("alpha", "first"),
        ("beta", "second"),
        ("gamma-long", "third"),
    ]
    .iter()
    .map(|(label, desc)| SelectItem {
        value: label.to_string(),
        label: label.to_string(),
        description: Some(desc.to_string()),
    })
    .collect()
}

#[test]
fn select_list_wide_pixels() {
    let mut list = SelectList::new(select_items(), 10);
    let lines = list.render(60);
    assert_eq!(
        stripped(&lines),
        vec![
            "→ alpha       first".to_string(),
            "  beta        second".to_string(),
            "  gamma-long  third".to_string(),
        ]
    );
    assert!(lines[0].contains(ACCENT), "selected label is accented");
    assert!(!lines[1].contains(ACCENT), "unselected label is plain");
    assert!(lines[0].contains(DIM), "description column is dim");
}

#[test]
fn select_list_narrow_drops_descriptions() {
    let mut list = SelectList::new(select_items(), 10);
    let lines = list.render(20);
    assert_eq!(
        stripped(&lines),
        vec![
            "→ alpha".to_string(),
            "  beta".to_string(),
            "  gamma-long".to_string(),
        ],
        "below the 10-cell minimum the description is dropped, label kept whole"
    );
}

#[test]
fn select_list_overflow_indicator_pixels() {
    let items: Vec<SelectItem> = ["a", "b", "c", "d", "e"]
        .iter()
        .map(|l| SelectItem {
            value: l.to_string(),
            label: l.to_string(),
            description: None,
        })
        .collect();
    let mut list = SelectList::new(items, 3);
    let lines = list.render(40);
    assert_eq!(lines.len(), 4, "3 windowed rows + indicator");
    assert_eq!(strip_ansi(&lines[3]), "  (1/5)");
    list.handle_input(&Key::Down);
    let lines = list.render(40);
    assert_eq!(strip_ansi(lines.last().unwrap()), "  (2/5)");
}

#[test]
fn select_list_empty_placeholder_pixels() {
    let mut list = SelectList::new(vec![], 10);
    let lines = list.render(40);
    assert_eq!(stripped(&lines), vec!["  No items".to_string()]);
    assert!(lines[0].contains(DIM));
}

// ── Slash-command autocomplete ───────────────────────────────────────────────

fn slash_commands() -> Vec<SlashCommand> {
    vec![
        SlashCommand {
            name: "model".into(),
            description: "Select model".into(),
        },
        SlashCommand {
            name: "clear".into(),
            description: "Clear history".into(),
        },
        SlashCommand {
            name: "quit".into(),
            description: "Exit TUI".into(),
        },
    ]
}

#[test]
fn autocomplete_windowed_pixels_with_overflow() {
    let mut ac = Autocomplete::new(slash_commands(), 2);
    ac.update("/");
    let lines = ac.render(60);
    assert_eq!(
        stripped(&lines),
        vec![
            "→ /model  Select model".to_string(),
            "  /clear  Clear history".to_string(),
            "  (1/3)".to_string(),
        ]
    );
    assert!(lines[0].contains(ACCENT), "selected /command is accented");
    assert!(lines[0].contains(DIM), "description is dim");
}

#[test]
fn autocomplete_narrow_truncates_whole_line() {
    let mut ac = Autocomplete::new(slash_commands(), 5);
    ac.update("/mo");
    let lines = ac.render(14);
    assert_eq!(
        strip_ansi(&lines[0]),
        "→ /model  Sele",
        "the inline description truncates with the line, it is not dropped"
    );
    assert!(visible_width(&lines[0]) <= 14);
}

// ── @files autocomplete ──────────────────────────────────────────────────────

#[test]
fn files_autocomplete_loaded_pixels() {
    let mut f =
        FilesAutocomplete::with_files(vec!["src/main.rs".to_string(), "src/lib.rs".to_string()], 5);
    f.update("@", 1);
    let lines = f.render(60);
    assert_eq!(
        stripped(&lines),
        vec!["→ @src/main.rs".to_string(), "  @src/lib.rs".to_string()]
    );
    assert!(lines[0].contains(ACCENT), "selected file is accented");
}

#[test]
fn files_autocomplete_loading_placeholder_pixels() {
    let mut f = FilesAutocomplete::new(5);
    f.update("@", 1);
    let lines = f.render(60);
    assert_eq!(stripped(&lines), vec!["→ loading files…".to_string()]);
    assert!(lines[0].contains(DIM), "loading placeholder is dim");
    assert!(
        !lines[0].contains(ACCENT),
        "loading placeholder is never accented"
    );
}

/// Rejected-attempt regression lock: while a STALE (but non-empty) file list
/// reloads in the background, the rows are the real files and the selection
/// keeps its `→` + accent — only the empty-list placeholder may render dim.
#[test]
fn files_autocomplete_stale_reload_keeps_selection_marker() {
    let mut f = FilesAutocomplete::with_files(vec!["src/main.rs".to_string()], 5);
    f.mark_loaded_at_for_test(Instant::now() - Duration::from_secs(31));
    f.update("@", 1);
    assert!(f.take_load_request(), "stale list should request a reload");
    let lines = f.render(60);
    assert_eq!(stripped(&lines), vec!["→ @src/main.rs".to_string()]);
    assert!(
        lines[0].contains(ACCENT),
        "selected row keeps its accent while reloading: {:?}",
        lines[0]
    );
    assert!(
        !lines[0].contains(DIM),
        "real rows are never dimmed by a background reload: {:?}",
        lines[0]
    );
}

#[test]
fn files_autocomplete_narrow_truncates_long_path() {
    let mut f = FilesAutocomplete::with_files(vec!["src/main.rs".to_string()], 5);
    f.update("@", 1);
    let lines = f.render(10);
    assert_eq!(
        strip_ansi(&lines[0]),
        "→ @src/mai",
        "a long path truncates with the line at narrow widths"
    );
    assert!(visible_width(&lines[0]) <= 10, "line must fit the width");
}

#[test]
fn files_autocomplete_overflow_indicator_pixels() {
    let files: Vec<String> = (0..5).map(|i| format!("f{i}.rs")).collect();
    let mut f = FilesAutocomplete::with_files(files, 3);
    f.update("@", 1);
    let lines = f.render(60);
    assert_eq!(lines.len(), 4);
    assert_eq!(strip_ansi(lines.last().unwrap()), "  (1/5)");
}

// ── Model selector ───────────────────────────────────────────────────────────

fn model_fixture() -> ModelSelector {
    let models = vec![
        ModelEntry {
            id: "a-model".to_string(),
            provider: "ProvA".to_string(),
            auth: None,
            is_current: false,
        },
        ModelEntry {
            id: "model-bb-long".to_string(),
            provider: "ProvB".to_string(),
            auth: None,
            is_current: false,
        },
    ];
    ModelSelector::with_models(models, Some("model-bb-long"))
}

#[test]
fn model_selector_wide_pixels_with_current_marker() {
    let mut sel = model_fixture();
    let lines = sel.render(60);
    assert_eq!(
        stripped(&lines),
        vec![
            "  Select Model (type to filter)".to_string(),
            "  Search: _".to_string(),
            String::new(),
            "  → a-model        ProvA".to_string(),
            // Current model has the LONGEST id: the ● marker sits after the
            // label, outside the alignment column, shifting the provider by
            // exactly the marker width — today's pixels.
            "    model-bb-long ●  ProvB".to_string(),
        ]
    );
    assert!(lines[3].contains(ACCENT), "selected model id is accented");
    assert!(lines[3].contains(DIM), "provider column is dim");
}

#[test]
fn model_selector_narrow_truncates_provider_not_drops_it() {
    let mut sel = model_fixture();
    let lines = sel.render(23);
    let plain = strip_ansi(&lines[3]);
    assert!(
        plain.starts_with("  → a-model        Pr"),
        "narrow width truncates the whole line, the provider is not dropped: {plain:?}"
    );
    for line in &lines {
        assert!(visible_width(line) <= 23, "line exceeds width: {line:?}");
    }
}

#[test]
fn model_selector_overflow_indicator_pixels() {
    let mut sel = ModelSelector::new(None);
    let lines = sel.render(80);
    let plain = strip_ansi(lines.last().unwrap());
    assert_eq!(
        plain, "  (1/28)",
        "12-row window over the 28 known models shows the indicator"
    );
}

/// Rejected-attempt regression lock: narrowing the filter CLAMPS the selection
/// into the new range (old semantics) — it must not reset it to row 0.
#[test]
fn model_selector_filter_change_clamps_selection() {
    let mut sel = ModelSelector::new(None);
    for _ in 0..5 {
        sel.handle_input(&Key::Down);
    }
    for c in "fireworks".chars() {
        sel.handle_input(&Key::Char(c));
    }
    assert_eq!(sel.visible_count(), 2, "two fireworks models match");
    let selected = sel.selected_model().expect("clamped selection").id.clone();
    // Clamped to the LAST match (index 1, kimi), not reset to row 0 (glm).
    assert!(
        selected.contains("kimi"),
        "selection must clamp to the last match, not reset to the first: {selected}"
    );
}
