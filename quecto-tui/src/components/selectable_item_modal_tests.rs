use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::components::ansi::strip_ansi;
use crate::components::utils::visible_width;

#[derive(Clone)]
struct FixtureItem {
    id: &'static str,
    label: &'static str,
    description: Option<&'static str>,
    metadata: Vec<&'static str>,
}

fn set(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| (*id).to_string()).collect()
}

fn fixture_items() -> Vec<FixtureItem> {
    vec![
        FixtureItem {
            id: "alpha",
            label: "Alpha Model",
            description: Some("OpenAI OAuth"),
            metadata: vec!["model", "fast"],
        },
        FixtureItem {
            id: "beta",
            label: "Beta Tool",
            description: Some("Filesystem access"),
            metadata: vec!["tool", "write", "uniquezz"],
        },
        FixtureItem {
            id: "gamma",
            label: "Gamma Flow",
            description: Some("Planning template"),
            metadata: vec!["flow", "plan", "gammaonly"],
        },
    ]
}

fn build_modal() -> SelectableItemModal {
    SelectableItemModal::builder()
        .items(fixture_items())
        .enabled_ids(set(&["alpha"]))
        .id(|item: &FixtureItem| item.id.to_string())
        .label(|item| item.label.to_string())
        .description(|item| item.description.map(str::to_string))
        .search_metadata(|item| item.metadata.iter().map(|s| (*s).to_string()).collect())
        .build()
        .unwrap()
}

fn plain_render(modal: &mut SelectableItemModal) -> String {
    modal
        .render(80)
        .into_iter()
        .map(|line| strip_ansi(&line))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn selectable_item_modal_filters_across_label_description_and_metadata() {
    let mut modal = build_modal();

    for c in "uniquezz".chars() {
        assert!(modal.handle_input(&Key::Char(c)));
    }

    assert_eq!(modal.visible_count(), 1);
    assert_eq!(modal.selected_item(), Some("beta"));
    let rendered = plain_render(&mut modal);
    assert!(rendered.contains("Beta Tool"), "{rendered}");
    assert!(!rendered.contains("Alpha Model"), "{rendered}");

    let mut modal = build_modal();
    for c in "OAuth".chars() {
        assert!(modal.handle_input(&Key::Char(c)));
    }
    assert_eq!(modal.visible_count(), 1);
    assert_eq!(modal.selected_item(), Some("alpha"));

    let mut modal = build_modal();
    for c in "Gamma".chars() {
        assert!(modal.handle_input(&Key::Char(c)));
    }
    assert_eq!(modal.visible_count(), 1);
    assert_eq!(modal.selected_item(), Some("gamma"));
}

#[test]
fn selectable_item_modal_shows_empty_state_and_backspace_clears_search() {
    let mut modal = build_modal();

    for c in "zzzz".chars() {
        modal.handle_input(&Key::Char(c));
    }
    assert_eq!(modal.visible_count(), 0);
    assert!(plain_render(&mut modal).contains("No matching items"));

    for _ in 0..4 {
        assert!(modal.handle_input(&Key::Backspace));
    }
    assert_eq!(modal.visible_count(), 3);
    assert!(plain_render(&mut modal).contains("Alpha Model"));
}

#[test]
fn selectable_item_modal_toggles_selected_item_and_applies_working_set() {
    let mut modal = build_modal();

    assert_eq!(modal.selected_item(), Some("alpha"));
    assert!(modal.handle_input(&Key::Char(' ')));
    assert!(modal.handle_input(&Key::Down));
    assert!(modal.handle_input(&Key::Char(' ')));
    assert!(modal.handle_input(&Key::Enter));

    assert_eq!(
        modal.take_result(),
        SelectableItemModalResult::Applied(set(&["beta"]))
    );
    assert_eq!(modal.take_result(), SelectableItemModalResult::Pending);
}

#[test]
fn selectable_item_modal_visible_bulk_actions_affect_only_filtered_items() {
    let mut modal = build_modal();

    for c in "uniquezz".chars() {
        modal.handle_input(&Key::Char(c));
    }
    assert_eq!(modal.selected_item(), Some("beta"));

    assert!(modal.handle_input(&Key::CtrlShift('a')));
    assert!(modal.handle_input(&Key::Enter));
    assert_eq!(
        modal.take_result(),
        SelectableItemModalResult::Applied(set(&["alpha", "beta"]))
    );

    let mut modal = build_modal();
    modal.handle_input(&Key::Char('m'));
    modal.handle_input(&Key::Char('o'));
    modal.handle_input(&Key::Char('d'));
    assert!(modal.handle_input(&Key::CtrlShift('d')));
    assert!(modal.handle_input(&Key::Enter));
    assert_eq!(
        modal.take_result(),
        SelectableItemModalResult::Applied(set(&[]))
    );
}

#[test]
fn selectable_item_modal_dismiss_restores_original_working_set() {
    let mut modal = build_modal();

    modal.handle_input(&Key::Char(' '));
    modal.handle_input(&Key::Escape);
    assert_eq!(modal.take_result(), SelectableItemModalResult::Dismissed);

    modal.handle_input(&Key::Enter);
    assert_eq!(
        modal.take_result(),
        SelectableItemModalResult::Applied(set(&["alpha"]))
    );
}

#[test]
fn selectable_item_modal_rejects_duplicate_ids() {
    let result = SelectableItemModal::builder()
        .items(vec!["same", "same"])
        .id(|item| (*item).to_string())
        .label(|item| (*item).to_string())
        .build();

    assert_eq!(
        result.unwrap_err(),
        SelectableItemModalError::DuplicateId("same".to_string())
    );
}

#[test]
fn selectable_item_modal_sanitizes_display_text_without_changing_canonical_ids() {
    let raw_id = "bad\x1b[31m-id";
    let mut modal = SelectableItemModal::builder()
        .items(vec![FixtureItem {
            id: raw_id,
            label: "Bad\x1b[31m Label",
            description: Some("Desc\nLine"),
            metadata: vec!["meta\x07data"],
        }])
        .enabled_ids(BTreeSet::new())
        .id(|item| item.id.to_string())
        .label(|item| item.label.to_string())
        .description(|item| item.description.map(str::to_string))
        .search_metadata(|item| item.metadata.iter().map(|s| (*s).to_string()).collect())
        .build()
        .unwrap();

    let raw_rendered = modal.render(80).join("\n");
    assert!(!raw_rendered.contains("\x1b[31m"), "{raw_rendered:?}");
    assert!(!raw_rendered.contains('\x07'), "{raw_rendered:?}");
    assert!(!raw_rendered.contains("Desc\nLine"), "{raw_rendered:?}");

    let rendered = strip_ansi(&raw_rendered);
    assert!(rendered.contains("Bad Label"), "{rendered}");
    assert!(rendered.contains("DescLine"), "{rendered}");

    assert!(modal.handle_input(&Key::Char(' ')));
    assert!(modal.handle_input(&Key::Enter));
    assert_eq!(
        modal.take_result(),
        SelectableItemModalResult::Applied(set(&[raw_id]))
    );
}

#[test]
fn selectable_item_modal_rejects_duplicate_raw_ids_without_collapsing_sanitized_ids() {
    let result = SelectableItemModal::builder()
        .items(vec!["ab", "a\u{202E}b"])
        .id(|item| (*item).to_string())
        .label(|item| (*item).to_string())
        .build()
        .unwrap();

    assert_eq!(result.visible_count(), 2);
}

#[test]
fn selectable_item_modal_clamps_selection_when_filter_removes_selected_row() {
    let mut modal = build_modal();

    modal.handle_input(&Key::Down);
    assert_eq!(modal.selected_item(), Some("beta"));
    for c in "gammaonly".chars() {
        modal.handle_input(&Key::Char(c));
    }

    assert_eq!(modal.visible_count(), 1);
    assert_eq!(modal.selected_item(), Some("gamma"));
}

#[test]
fn selectable_item_modal_handle_input_consumption_and_query_bound() {
    let mut modal = build_modal();

    assert!(!modal.handle_input(&Key::Tab));
    for _ in 0..MAX_QUERY_LEN + 5 {
        assert!(modal.handle_input(&Key::Char('a')));
    }
    let rendered = plain_render(&mut modal);
    let search_line = rendered
        .lines()
        .find(|line| line.contains("Search:"))
        .unwrap();
    let typed_query_len = search_line
        .trim_start_matches("  Search: ")
        .trim_end_matches('_')
        .chars()
        .count();
    assert_eq!(typed_query_len, MAX_QUERY_LEN);
}

#[test]
fn selectable_item_modal_overlay_renders_frame_footer_and_width_bounded_lines() {
    let mut modal = build_modal();
    let (lines, panel_width) = build_selectable_item_modal_overlay(
        "Manage items",
        "Space toggle · Ctrl+Shift+A enable visible · Ctrl+Shift+D disable visible · Enter apply · Esc dismiss",
        &mut modal,
        50,
        20,
    );
    let plain = lines
        .iter()
        .map(|line| strip_ansi(line))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(plain.contains("Manage items"), "{plain}");
    assert!(plain.contains("Space toggle"), "{plain}");
    assert!(plain.contains("[x] Alpha Model"), "{plain}");
    assert!(plain.contains("[ ] Beta Tool"), "{plain}");
    assert!(panel_width <= 50);
    assert!(lines.iter().all(|line| visible_width(line) <= 50));
}

#[derive(Clone)]
struct ModelLike {
    id: String,
    provider: String,
}
#[derive(Clone)]
struct ToolLike {
    name: String,
    mode: String,
}
#[derive(Clone)]
struct WorkflowLike {
    key: String,
    phase: String,
}

#[test]
fn selectable_item_modal_builder_supports_model_tool_and_workflow_shapes() {
    let mut model_modal = SelectableItemModal::builder()
        .items(vec![ModelLike {
            id: "openai/gpt".into(),
            provider: "OpenAI".into(),
        }])
        .id(|item| item.id.clone())
        .label(|item| item.id.clone())
        .search_metadata(|item| vec![item.provider.clone()])
        .build()
        .unwrap();
    for c in "openai".chars() {
        model_modal.handle_input(&Key::Char(c));
    }
    assert_eq!(model_modal.selected_item(), Some("openai/gpt"));

    let mut tool_modal = SelectableItemModal::builder()
        .items(vec![ToolLike {
            name: "bash".into(),
            mode: "write-capable".into(),
        }])
        .id(|item| item.name.clone())
        .label(|item| item.name.clone())
        .description(|item| Some(item.mode.clone()))
        .build()
        .unwrap();
    for c in "write".chars() {
        tool_modal.handle_input(&Key::Char(c));
    }
    assert_eq!(tool_modal.selected_item(), Some("bash"));

    let mut workflow_modal = SelectableItemModal::builder()
        .items(vec![WorkflowLike {
            key: "feature".into(),
            phase: "green".into(),
        }])
        .id(|item| item.key.clone())
        .label(|item| item.key.clone())
        .search_metadata(|item| vec![item.phase.clone()])
        .build()
        .unwrap();
    for c in "green".chars() {
        workflow_modal.handle_input(&Key::Char(c));
    }
    assert_eq!(workflow_modal.selected_item(), Some("feature"));
}

#[derive(Default)]
struct FixtureProvider {
    applied: Option<BTreeSet<String>>,
    dismissed: bool,
}

impl SelectableItemProvider for FixtureProvider {
    type Item = FixtureItem;

    fn selectable_items(&self) -> Vec<Self::Item> {
        fixture_items()
    }

    fn enabled_item_ids(&self) -> BTreeSet<String> {
        set(&["gamma"])
    }

    fn item_id(&self, item: &Self::Item) -> String {
        item.id.to_string()
    }

    fn item_label(&self, item: &Self::Item) -> String {
        item.label.to_string()
    }

    fn item_description(&self, item: &Self::Item) -> Option<String> {
        item.description.map(str::to_string)
    }

    fn item_search_metadata(&self, item: &Self::Item) -> Vec<String> {
        item.metadata.iter().map(|s| (*s).to_string()).collect()
    }

    fn apply_enabled_item_ids(&mut self, enabled_ids: BTreeSet<String>) {
        self.applied = Some(enabled_ids);
    }

    fn dismiss_selectable_items(&mut self) {
        self.dismissed = true;
    }
}

#[test]
fn selectable_item_modal_can_build_from_provider_contract() {
    let mut provider = FixtureProvider::default();
    let mut modal = SelectableItemModal::from_provider(&provider).unwrap();
    modal.handle_input(&Key::Enter);
    if let SelectableItemModalResult::Applied(ids) = modal.take_result() {
        provider.apply_enabled_item_ids(ids);
    }
    assert_eq!(provider.applied, Some(set(&["gamma"])));
    assert!(!provider.dismissed);

    let mut modal = SelectableItemModal::from_provider(&provider).unwrap();
    modal.handle_input(&Key::Escape);
    if modal.take_result() == SelectableItemModalResult::Dismissed {
        provider.dismiss_selectable_items();
    }
    assert!(provider.dismissed);
}

#[test]
fn selectable_item_modal_cycles_four_state_scope_rows_and_applies_scopes() {
    let mut scopes = BTreeMap::new();
    scopes.insert("alpha".to_string(), ScopeSelection::None);
    let mut modal = SelectableItemModal::builder()
        .items(fixture_items())
        .id(|item: &FixtureItem| item.id.to_string())
        .label(|item| item.label.to_string())
        .description(|item| item.description.map(str::to_string))
        .build()
        .unwrap()
        .with_scope_selection(scopes);

    assert!(plain_render(&mut modal).contains("[--] Alpha Model"));
    assert!(modal.handle_input(&Key::Char(' ')));
    assert!(plain_render(&mut modal).contains("[P-] Alpha Model"));
    assert!(modal.handle_input(&Key::Char(' ')));
    assert!(plain_render(&mut modal).contains("[-C] Alpha Model"));
    assert!(modal.handle_input(&Key::Char(' ')));
    assert!(plain_render(&mut modal).contains("[PC] Alpha Model"));
    assert!(modal.handle_input(&Key::Char(' ')));
    assert!(plain_render(&mut modal).contains("[--] Alpha Model"));

    assert!(modal.handle_input(&Key::Char(' ')));
    assert!(modal.handle_input(&Key::Enter));
    assert_eq!(
        modal.take_result(),
        SelectableItemModalResult::AppliedScopes(BTreeMap::from([(
            "alpha".to_string(),
            ScopeSelection::Parent,
        )]))
    );
}

#[test]
fn selectable_item_modal_scope_mode_bulk_actions_update_visible_scopes() {
    let mut modal = SelectableItemModal::builder()
        .items(fixture_items())
        .id(|item: &FixtureItem| item.id.to_string())
        .label(|item| item.label.to_string())
        .description(|item| item.description.map(str::to_string))
        .search_metadata(|item| item.metadata.iter().map(|s| (*s).to_string()).collect())
        .build()
        .unwrap()
        .with_scope_selection(BTreeMap::from([
            ("alpha".to_string(), ScopeSelection::None),
            ("beta".to_string(), ScopeSelection::Child),
            ("gamma".to_string(), ScopeSelection::Both),
        ]));

    for c in "uniquezz".chars() {
        modal.handle_input(&Key::Char(c));
    }
    assert_eq!(modal.selected_item(), Some("beta"));

    assert!(modal.handle_input(&Key::CtrlShift('a')));
    assert!(modal.handle_input(&Key::Enter));
    assert_eq!(
        modal.take_result(),
        SelectableItemModalResult::AppliedScopes(BTreeMap::from([
            ("alpha".to_string(), ScopeSelection::None),
            ("beta".to_string(), ScopeSelection::Both),
            ("gamma".to_string(), ScopeSelection::Both),
        ]))
    );

    let mut modal = SelectableItemModal::builder()
        .items(fixture_items())
        .id(|item: &FixtureItem| item.id.to_string())
        .label(|item| item.label.to_string())
        .description(|item| item.description.map(str::to_string))
        .search_metadata(|item| item.metadata.iter().map(|s| (*s).to_string()).collect())
        .build()
        .unwrap()
        .with_scope_selection(BTreeMap::from([
            ("alpha".to_string(), ScopeSelection::Parent),
            ("beta".to_string(), ScopeSelection::Both),
            ("gamma".to_string(), ScopeSelection::Child),
        ]));

    for c in "gammaonly".chars() {
        modal.handle_input(&Key::Char(c));
    }
    assert_eq!(modal.selected_item(), Some("gamma"));

    assert!(modal.handle_input(&Key::CtrlShift('d')));
    assert!(modal.handle_input(&Key::Enter));
    assert_eq!(
        modal.take_result(),
        SelectableItemModalResult::AppliedScopes(BTreeMap::from([
            ("alpha".to_string(), ScopeSelection::Parent),
            ("beta".to_string(), ScopeSelection::Both),
            ("gamma".to_string(), ScopeSelection::None),
        ]))
    );
}

#[test]
fn selectable_item_modal_wide_render_uses_two_columns_for_many_items() {
    let items = (0..16)
        .map(|idx| FixtureItem {
            id: Box::leak(format!("tool-{idx:02}").into_boxed_str()),
            label: Box::leak(format!("Tool {idx:02}").into_boxed_str()),
            description: Some("kernel tool"),
            metadata: vec!["tool"],
        })
        .collect::<Vec<_>>();
    let mut modal = SelectableItemModal::builder()
        .items(items)
        .enabled_ids(BTreeSet::new())
        .id(|item: &FixtureItem| item.id.to_string())
        .label(|item| item.label.to_string())
        .description(|item| item.description.map(str::to_string))
        .search_metadata(|item| item.metadata.iter().map(|s| (*s).to_string()).collect())
        .build()
        .unwrap();

    let rendered = modal
        .render(100)
        .into_iter()
        .map(|line| strip_ansi(&line))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        rendered
            .lines()
            .any(|line| line.contains("Tool 00") && line.contains("Tool 08")),
        "wide modal should pack the visible tool list into two columns:\n{rendered}"
    );
    modal.handle_input(&Key::CtrlShift('a'));
    modal.handle_input(&Key::Enter);
    assert_eq!(
        modal.take_result(),
        SelectableItemModalResult::Applied(set(&[
            "tool-00", "tool-01", "tool-02", "tool-03", "tool-04", "tool-05", "tool-06", "tool-07",
            "tool-08", "tool-09", "tool-10", "tool-11", "tool-12", "tool-13", "tool-14", "tool-15",
        ]))
    );
}

fn many_tool_modal(count: usize) -> SelectableItemModal {
    let items = (0..count)
        .map(|idx| FixtureItem {
            id: Box::leak(format!("tool-{idx:02}").into_boxed_str()),
            label: Box::leak(format!("Tool {idx:02}").into_boxed_str()),
            description: Some("kernel tool"),
            metadata: vec![Box::leak(format!("group-{idx:02}").into_boxed_str())],
        })
        .collect::<Vec<_>>();
    SelectableItemModal::builder()
        .items(items)
        .enabled_ids(BTreeSet::new())
        .id(|item: &FixtureItem| item.id.to_string())
        .label(|item| item.label.to_string())
        .description(|item| item.description.map(str::to_string))
        .search_metadata(|item| item.metadata.iter().map(|s| (*s).to_string()).collect())
        .build()
        .unwrap()
}

fn rendered_plain(modal: &mut SelectableItemModal, width: usize) -> String {
    modal
        .render(width)
        .into_iter()
        .map(|line| strip_ansi(&line))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn selectable_item_modal_two_column_layout_boundaries_are_pinned() {
    let mut twelve = many_tool_modal(12);
    let twelve_rendered = rendered_plain(&mut twelve, 100);
    assert!(
        !twelve_rendered
            .lines()
            .any(|line| line.contains("Tool 00") && line.contains("Tool 06")),
        "12 visible items should remain a single column:\n{twelve_rendered}"
    );

    let mut thirteen_wide = many_tool_modal(13);
    let thirteen_wide_rendered = rendered_plain(&mut thirteen_wide, 100);
    assert!(
        thirteen_wide_rendered
            .lines()
            .any(|line| line.contains("Tool 00") && line.contains("Tool 07")),
        "13 visible items should use two columns on a wide modal:\n{thirteen_wide_rendered}"
    );

    let mut thirteen_narrow = many_tool_modal(13);
    let thirteen_narrow_rendered = rendered_plain(&mut thirteen_narrow, 71);
    assert!(
        !thirteen_narrow_rendered
            .lines()
            .any(|line| line.contains("Tool 00") && line.contains("Tool 07")),
        "width 71 should stay single-column:\n{thirteen_narrow_rendered}"
    );

    let mut thirteen_at_threshold = many_tool_modal(13);
    let thirteen_at_threshold_rendered = rendered_plain(&mut thirteen_at_threshold, 72);
    assert!(
        thirteen_at_threshold_rendered
            .lines()
            .any(|line| line.contains("Tool 00") && line.contains("Tool 07")),
        "width 72 should enable two-column layout:\n{thirteen_at_threshold_rendered}"
    );
}

#[test]
fn selectable_item_modal_two_column_mode_preserves_filter_navigation_toggle_and_bulk_disable() {
    let mut modal = many_tool_modal(16);

    modal.handle_input(&Key::Down);
    let navigated = rendered_plain(&mut modal, 100);
    assert!(
        navigated.lines().any(|line| line.contains("→ [ ] Tool 01")),
        "selection should move while rendered in two columns:\n{navigated}"
    );

    modal.handle_input(&Key::Char(' '));
    modal.handle_input(&Key::CtrlShift('a'));
    modal.handle_input(&Key::CtrlShift('d'));
    modal.handle_input(&Key::Enter);
    assert_eq!(
        modal.take_result(),
        SelectableItemModalResult::Applied(set(&[]))
    );

    let mut filtered = many_tool_modal(16);
    for c in "group-15".chars() {
        filtered.handle_input(&Key::Char(c));
    }
    assert_eq!(filtered.visible_count(), 1);
    assert_eq!(filtered.selected_item(), Some("tool-15"));
    filtered.handle_input(&Key::CtrlShift('a'));
    filtered.handle_input(&Key::Enter);
    assert_eq!(
        filtered.take_result(),
        SelectableItemModalResult::Applied(set(&["tool-15"]))
    );
}

#[test]
fn selectable_item_modal_two_column_visible_window_caps_at_twenty_four_items() {
    let mut modal = many_tool_modal(25);
    let rendered = rendered_plain(&mut modal, 100);

    assert!(
        rendered.contains("Tool 23"),
        "24th visible row missing:\n{rendered}"
    );
    assert!(
        !rendered.contains("Tool 24"),
        "25th row should be paged out:\n{rendered}"
    );
    assert!(
        rendered.contains("(1/25)"),
        "overflow indicator missing:\n{rendered}"
    );
}
