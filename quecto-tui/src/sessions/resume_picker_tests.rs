use crate::components::{
    component::Component,
    select_list::{SelectItem, SelectList},
};
use crate::shell::keys::{Key, parse_key};
fn item(id: &str) -> SelectItem {
    SelectItem {
        value: id.into(),
        label: id.into(),
        description: Some("/execution/path".into()),
    }
}
#[test]
fn baseline_focus_keys_are_available_to_feature() {
    let mut list = SelectList::new(vec![item("a")], 10);
    assert!(!list.handle_input(&Key::Tab));
    assert!(!list.handle_input(&Key::BackTab));
    assert_eq!(parse_key(b"\x1b[Z").unwrap().0, Key::BackTab);
}
#[test]
fn baseline_mouse_decode_is_zero_based() {
    assert_eq!(
        parse_key(b"\x1b[<0;12;8M").unwrap().0,
        Key::MousePress(11, 7)
    );
}
#[test]
fn baseline_async_refresh_preserves_stable_identity() {
    let mut list = SelectList::new(vec![item("a"), item("b")], 10);
    list.handle_input(&Key::Down);
    list.sync_items(vec![item("b"), item("a")]);
    assert_eq!(list.selected_item().unwrap().value, "b");
    assert!(list.render_text(70).contains("/execution/path"));
}

use super::resume_picker::{ResumePicker, ResumePickerEvent};
use crate::protocol::session_payloads::SessionListScope;
#[test]
fn scope_focus_and_query_cycle_reporting_the_search_text_without_filtering() {
    let mut picker = ResumePicker::new(vec![item("a"), item("b")], SessionListScope::Local);
    picker.handle_input(&Key::Tab);
    assert_eq!(
        picker.handle_input(&Key::Char(' ')),
        ResumePickerEvent::ScopeChanged(SessionListScope::Global)
    );
    picker.sync_items(vec![item("a"), item("b")]);
    picker.handle_input(&Key::Tab);
    assert_eq!(
        picker.handle_input(&Key::Char('b')),
        ResumePickerEvent::QueryChanged("b".into())
    );
    // The picker filters nothing: both rows stay until an answer replaces them.
    assert_eq!((picker.item_count(), picker.query()), (2, "b"));
    picker.sync_items(vec![item("b")]);
    picker.handle_input(&Key::Tab);
    assert_eq!(
        picker.handle_input(&Key::Enter),
        ResumePickerEvent::Selected("b".into())
    );
    picker.handle_input(&Key::BackTab);
    assert_eq!(
        picker.handle_input(&Key::Backspace),
        ResumePickerEvent::QueryChanged(String::new())
    );
    assert_eq!(
        picker.handle_input(&Key::Backspace),
        ResumePickerEvent::Pending,
        "nothing to erase"
    );
    assert_eq!(
        picker.handle_input(&Key::Char('/')),
        ResumePickerEvent::QueryChanged("/".into())
    );
    assert_eq!(
        picker.handle_input(&Key::Char('\u{1b}')),
        ResumePickerEvent::Pending
    );
    assert_eq!(
        picker.handle_input(&Key::Char('\u{202e}')),
        ResumePickerEvent::Pending
    );
    picker.sync_items(Vec::new());
    assert!(picker.selected_item().is_none());
    assert_eq!(picker.scope(), SessionListScope::Global);
    assert_eq!(
        picker.handle_input(&Key::Escape),
        ResumePickerEvent::Dismissed
    );
}
#[test]
fn empty_picker_scope_mouse_is_centered_and_visible() {
    let mut picker = ResumePicker::default();
    let (lines, width) = picker.render(100, 30);
    let scope_row = lines
        .iter()
        .position(|s| s.contains("Local Folder") && s.contains("All Folders"))
        .unwrap();
    let x = (100 - width) / 2 + column_of(&lines[scope_row], "All Folders");
    let y = (30 - lines.len()) / 2 + scope_row;
    assert_eq!(
        picker.handle_input(&Key::MousePress(x as u16, y as u16)),
        ResumePickerEvent::ScopeChanged(SessionListScope::Global)
    );
    assert_eq!(
        picker.handle_input(&Key::Enter),
        ResumePickerEvent::ScopeChanged(SessionListScope::Local)
    );
}

#[test]
fn refresh_preserves_selection_and_scope_change_clears_actionable_rows() {
    let mut picker = ResumePicker::new(vec![item("a"), item("b")], SessionListScope::Local);
    picker.handle_input(&Key::Down);
    picker.sync_items(vec![item("b"), item("a")]);
    assert_eq!(picker.selected_item().unwrap().value, "b");
    picker.handle_input(&Key::Tab);
    picker.handle_input(&Key::Right);
    picker.handle_input(&Key::BackTab);
    assert_eq!(picker.handle_input(&Key::Enter), ResumePickerEvent::Pending);
    assert_eq!(picker.item_count(), 0);
}
#[test]
fn mouse_result_uses_visible_window_and_ignores_outside_clicks() {
    let items = (0..20).map(|i| item(&i.to_string())).collect();
    let mut picker = ResumePicker::new(items, SessionListScope::Local);
    for _ in 0..15 {
        picker.handle_input(&Key::Down);
    }
    let (lines, width) = picker.render(100, 40);
    let x = (100 - width) / 2 + 3;
    // The first list row sits directly under the `Sessions` header.
    let sessions = lines.iter().position(|l| l.contains("Sessions")).unwrap();
    let y = (40 - lines.len()) / 2 + sessions + 1;
    assert_eq!(
        picker.handle_input(&Key::MousePress(0, 0)),
        ResumePickerEvent::Pending
    );
    assert_eq!(
        picker.handle_input(&Key::MousePress(x as u16, y as u16)),
        ResumePickerEvent::Selected("4".into())
    );
}
#[test]
fn safe_rows_preserve_opaque_identity_and_small_terminals_do_not_panic() {
    let mut hostile = item("opaque\x1bkey");
    hostile.label = "hello\n\x1b[31mworld".into();
    hostile.description = Some("/tmp\r\t\x07path".into());
    let mut picker = ResumePicker::new(vec![hostile], SessionListScope::Local);
    assert_eq!(picker.selected_item().unwrap().value, "opaque\x1bkey");
    assert!(
        picker
            .selected_item()
            .unwrap()
            .label
            .chars()
            .all(|c| c >= ' ')
    );
    for width in 0..10 {
        for height in 0..10 {
            picker.render(width, height);
            picker.handle_input(&Key::MousePress(0, 0));
        }
    }
}

#[test]
fn selected_details_show_long_path_suffix_and_unavailable_actions() {
    let mut row = item("a");
    row.description = Some(format!(
        "/{} /distinct-worktree — Open original / Fork / Locate unavailable; Cancel",
        "long-path/".repeat(18)
    ));
    let mut picker = ResumePicker::new(vec![row], SessionListScope::Global);
    let (lines, width) = picker.render_overlay(180, 40);
    let text = lines.join("\n");
    assert!(text.contains("/distinct-worktree"), "{text}");
    assert!(text.contains("Locate unavailable"), "{text}");
    assert!(text.contains("Cancel"));
    let x = (180 - width) / 2 + 3;
    let details = lines
        .iter()
        .position(|l| l.contains("Locate unavailable"))
        .unwrap();
    let y = (40 - lines.len()) / 2 + details;
    assert_eq!(
        picker.handle_key(&Key::MousePress(x as u16, y as u16)),
        ResumePickerEvent::Pending
    );
    assert_eq!(
        picker.handle_key(&Key::Ctrl('g')),
        ResumePickerEvent::Pending
    );
    assert_eq!(picker.selected_item().unwrap().value, "a");
}

#[test]
fn short_height_selected_result_is_visible_before_keyboard_and_mouse_activation() {
    let items = (0..20).map(|i| item(&format!("ROW-{i:02}"))).collect();
    let mut picker = ResumePicker::new(items, SessionListScope::Local);
    picker.render_overlay(100, 18);
    for _ in 0..10 {
        picker.handle_key(&Key::Down);
    }
    let (lines, width) = picker.render_overlay(100, 18);
    let text = lines.join("\n");
    // Independently reproduced against always-12: overlay truncates to
    // height-4=14, so ROW-10 sits past the clip while Enter still returned it.
    // A height-derived window must scroll ROW-00 off and keep ROW-10 on-screen.
    assert!(text.contains("ROW-10"), "{text}");
    assert!(!text.contains("ROW-00"), "{text}");
    let result_identities = (0..20)
        .filter(|i| text.contains(&format!("ROW-{i:02}")))
        .count();
    assert!(
        result_identities > 0 && result_identities < 12,
        "visible results must be fitted to height 18, not always-12: {result_identities} {text}"
    );
    assert!(text.contains("Esc close"));
    assert_eq!(
        picker.handle_key(&Key::Enter),
        ResumePickerEvent::Selected("ROW-10".into())
    );
    picker.handle_key(&Key::ScrollDown);
    let (lines, _) = picker.render_overlay(100, 18);
    assert_eq!(picker.selected_item().unwrap().value, "ROW-11");
    assert!(lines.join("\n").contains("ROW-11"));
    let first_row = lines.iter().position(|line| line.contains("ROW-")).unwrap();
    let expected = (0..20)
        .map(|i| format!("ROW-{i:02}"))
        .find(|id| lines[first_row].contains(id))
        .unwrap();
    let x = (100 - width) / 2 + 3;
    let y = (18 - lines.len()) / 2 + first_row;
    assert_eq!(
        picker.handle_key(&Key::MousePress(x as u16, y as u16)),
        ResumePickerEvent::Selected(expected)
    );
}

#[test]
fn terminal_without_result_space_cannot_activate_hidden_selection() {
    let mut picker = ResumePicker::new(vec![item("ROW-00")], SessionListScope::Local);
    picker.render_overlay(100, 8);
    assert_eq!(picker.handle_key(&Key::Enter), ResumePickerEvent::Pending);
    assert_eq!(
        picker.handle_key(&Key::Char(' ')),
        ResumePickerEvent::Pending
    );
}

#[test]
fn resizing_and_wrapping_navigation_keep_selection_visible_and_details_inert() {
    let items = (0..20).map(|i| item(&format!("ROW-{i:02}"))).collect();
    let mut picker = ResumePicker::new(items, SessionListScope::Local);
    for height in [40, 18, 12, 30] {
        for key in [Key::Up, Key::Down, Key::ScrollUp, Key::ScrollDown] {
            picker.render_overlay(100, height);
            picker.handle_key(&key);
            let (lines, width) = picker.render_overlay(100, height);
            let selected = picker.selected_item().unwrap().value.clone();
            assert!(lines.join("\n").contains(&selected), "height={height}");
            let footer = lines
                .iter()
                .position(|line| line.contains("Esc close"))
                .unwrap();
            let x = (100 - width) / 2 + 3;
            let y = (height - lines.len()) / 2 + footer;
            assert_eq!(
                picker.handle_key(&Key::MousePress(x as u16, y as u16)),
                ResumePickerEvent::Pending
            );
            assert_eq!(
                picker.handle_key(&Key::Enter),
                ResumePickerEvent::Selected(selected)
            );
        }
    }
}

#[test]
fn overflow_indicator_row_is_not_an_activatable_result() {
    let items = (0..20).map(|i| item(&format!("ROW-{i:02}"))).collect();
    let mut picker = ResumePicker::new(items, SessionListScope::Local);
    let (lines, width) = picker.render_overlay(100, 18);
    let indicator = lines
        .iter()
        .position(|line| line.contains("(1/20)"))
        .expect("overflow indicator");
    let x = (100 - width) / 2 + 3;
    let y = (18 - lines.len()) / 2 + indicator;
    assert_eq!(
        picker.handle_key(&Key::MousePress(x as u16, y as u16)),
        ResumePickerEvent::Pending
    );
    assert_eq!(picker.selected_item().unwrap().value, "ROW-00");
}

use crate::components::ansi::strip_ansi;
use crate::components::utils::visible_width;

/// Column of `needle` in `line` as the terminal shows it (ANSI stripped).
fn column_of(line: &str, needle: &str) -> usize {
    let plain = strip_ansi(line);
    visible_width(&plain[..plain.find(needle).unwrap()])
}

#[test]
fn focus_marker_moves_between_sessions_scope_and_search() {
    let mut picker = ResumePicker::new(vec![item("a"), item("b")], SessionListScope::Local);
    let marked = |lines: &[String]| -> Vec<String> {
        lines
            .iter()
            .map(|l| strip_ansi(l).trim_matches(['│', ' ']).to_string())
            .filter(|l| l.starts_with("▸ "))
            .collect()
    };
    let (lines, _) = picker.render(100, 30);
    let text = lines.join("\n");
    assert_eq!(marked(&lines).len(), 1, "{text}");
    assert!(marked(&lines)[0].contains("▸ Sessions"), "{text}");
    assert!(
        text.contains("→ \x1b[36ma"),
        "active selection accented: {text}"
    );
    assert!(
        !text.contains('▏'),
        "no text cursor without Search focus: {text}"
    );

    picker.handle_input(&Key::Tab);
    let (lines, _) = picker.render(100, 30);
    let text = lines.join("\n");
    assert_eq!(marked(&lines).len(), 1, "{text}");
    assert!(
        marked(&lines)[0].contains("▸ Scope:   [Local Folder]   All Folders"),
        "{text}"
    );
    assert!(
        text.contains("\x1b[2m→ "),
        "inactive selection dimmed: {text}"
    );
    assert!(!text.contains("→ \x1b[36ma"), "{text}");

    picker.handle_input(&Key::Tab);
    picker.handle_input(&Key::Char('x'));
    let (lines, _) = picker.render(100, 30);
    let text = lines.join("\n");
    assert_eq!(marked(&lines).len(), 1, "{text}");
    assert!(marked(&lines)[0].contains("▸ Search:  x▏"), "{text}");
    assert!(!text.contains("Query:"), "{text}");
}

#[test]
fn footer_names_sections_in_plain_words_and_highlights_the_focused_one() {
    let mut picker = ResumePicker::new(vec![item("a")], SessionListScope::Local);
    let footer = |picker: &mut ResumePicker| {
        let (lines, _) = picker.render(100, 30);
        lines
            .iter()
            .find(|l| l.contains("Esc close"))
            .cloned()
            .unwrap_or_else(|| panic!("footer: {}", lines.join("\n")))
    };
    let line = footer(&mut picker);
    assert_eq!(
        strip_ansi(&line).trim_matches(['│', ' ']),
        "Tab: Sessions ▸ Scope ▸ Search · ↑↓ · Enter open · Esc close"
    );
    assert!(line.contains("\x1b[36mSessions\x1b[0m"), "{line:?}");
    picker.handle_input(&Key::Tab);
    assert!(footer(&mut picker).contains("\x1b[36mScope\x1b[0m"));
    picker.handle_input(&Key::Tab);
    assert!(footer(&mut picker).contains("\x1b[36mSearch\x1b[0m"));
}

#[test]
fn scope_switch_clicks_land_on_the_rendered_labels() {
    let mut picker = ResumePicker::default();
    let (lines, width) = picker.render(100, 30);
    let scope_row = lines.iter().position(|l| l.contains("Scope:")).unwrap();
    let left = (100 - width) / 2;
    let top = (30 - lines.len()) / 2;
    // Last cell of the inactive label.
    let x = left + column_of(&lines[scope_row], "All Folders") + "All Folders".len() - 1;
    assert_eq!(
        picker.handle_input(&Key::MousePress(x as u16, (top + scope_row) as u16)),
        ResumePickerEvent::ScopeChanged(SessionListScope::Global)
    );
    let (lines, _) = picker.render(100, 30);
    assert!(strip_ansi(&lines[scope_row]).contains("Scope:    Local Folder   [All Folders]"));
    let x = left + column_of(&lines[scope_row], "Local Folder") + 4;
    assert_eq!(
        picker.handle_input(&Key::MousePress(x as u16, (top + scope_row) as u16)),
        ResumePickerEvent::ScopeChanged(SessionListScope::Local)
    );
    let x = left + column_of(&lines[scope_row], "Scope:");
    assert_eq!(
        picker.handle_input(&Key::MousePress(x as u16, (top + scope_row) as u16)),
        ResumePickerEvent::Pending
    );
}

#[test]
fn empty_list_keeps_its_placeholder_under_every_focus() {
    let mut picker = ResumePicker::default();
    for _ in 0..3 {
        let (lines, _) = picker.render(100, 30);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        let sessions = plain.iter().position(|l| l.contains("Sessions")).unwrap();
        assert!(
            plain[sessions + 1].starts_with("│       No items"),
            "{}",
            plain.join("\n")
        );
        picker.handle_input(&Key::Tab);
    }
}

/// Content rows as the terminal shows them: ANSI stripped, border and its
/// one-cell padding removed, trailing padding trimmed.
fn content_rows(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|l| strip_ansi(l))
        .filter(|l| l.starts_with('│'))
        .map(|l| {
            l.trim_start_matches('│')
                .strip_prefix(' ')
                .unwrap_or_default()
                .trim_end_matches([' ', '│'])
                .to_string()
        })
        .collect()
}

#[test]
fn layout_separates_sections_with_blank_rows_and_indents_list_rows() {
    let mut a = item("write a story about a cat");
    a.description = Some("/home/swq/thoughts · 2026-09-17 14:57 (2 msgs) · Resume".into());
    let mut b = item("whats in this repo?");
    b.description = Some("/home/swq/thoughts · 2026-09-16".into());
    let mut picker = ResumePicker::new(vec![a, b], SessionListScope::Local);
    let (lines, _) = picker.render(100, 30);
    let rows = content_rows(&lines);
    assert_eq!(rows[0], "Resume session", "{rows:?}");
    assert_eq!(rows[1], "", "{rows:?}");
    assert_eq!(
        rows[2], "  Scope:   [Local Folder]   All Folders",
        "{rows:?}"
    );
    assert_eq!(rows[3], "  Search:", "{rows:?}");
    assert_eq!(rows[4], "", "{rows:?}");
    assert_eq!(rows[5], "▸ Sessions", "{rows:?}");
    assert!(
        rows[6].starts_with("    → write a story about a cat"),
        "{rows:?}"
    );
    assert!(rows[7].starts_with("      whats in this repo?"), "{rows:?}");
    assert_eq!(rows[8], "", "{rows:?}");
    assert_eq!(
        rows[9], "  /home/swq/thoughts · 2026-09-17 14:57 (2 msgs) · Resume",
        "{rows:?}"
    );
    assert_eq!(rows[10], "", "{rows:?}");
    assert_eq!(
        rows[11], "  Tab: Sessions ▸ Scope ▸ Search · ↑↓ · Enter open · Esc close",
        "{rows:?}"
    );
    assert_eq!(rows.len(), 12, "{rows:?}");

    picker.handle_input(&Key::Tab);
    picker.handle_input(&Key::Right);
    picker.sync_items(vec![]);
    let (lines, _) = picker.render(100, 30);
    let rows = content_rows(&lines);
    assert_eq!(
        rows[2], "▸ Scope:    Local Folder   [All Folders]",
        "{rows:?}"
    );
    assert_eq!(rows[5], "  Sessions", "{rows:?}");
    assert_eq!(rows[6], "      No items", "{rows:?}");
    assert_eq!(rows[7], "", "{rows:?}");
    assert_eq!(
        rows[8], "  Tab: Sessions ▸ Scope ▸ Search · ↑↓ · Enter open · Esc close",
        "{rows:?}"
    );
    assert_eq!(rows.len(), 9, "{rows:?}");
}

#[test]
fn short_terminals_drop_blank_rows_before_result_rows() {
    use super::resume_picker::fit_result_window;
    // Tall: results, both gaps and the details all fit.
    let fit = fit_result_window(20, 2, 1);
    assert_eq!((fit.result_rows, fit.indicator, fit.detail_rows), (2, 0, 1));
    assert_eq!((fit.gap_before_details, fit.gap_before_footer), (1, 1));
    // One spare row: the details gap survives, the footer gap goes.
    let fit = fit_result_window(4, 2, 1);
    assert_eq!((fit.gap_before_details, fit.gap_before_footer), (1, 0));
    // No spare row: no gaps, every result row kept.
    let fit = fit_result_window(3, 2, 1);
    assert_eq!((fit.result_rows, fit.detail_rows), (2, 1));
    assert_eq!((fit.gap_before_details, fit.gap_before_footer), (0, 0));
    // Overflowing list: the indicator and results win over the gaps.
    let fit = fit_result_window(5, 20, 1);
    assert_eq!((fit.result_rows, fit.indicator, fit.detail_rows), (3, 1, 1));
    assert_eq!((fit.gap_before_details, fit.gap_before_footer), (0, 0));
    // No details: the only gap sits before the footer.
    let fit = fit_result_window(3, 1, 0);
    assert_eq!((fit.gap_before_details, fit.gap_before_footer), (0, 1));
    assert_eq!(fit_result_window(0, 5, 3).result_rows, 0);

    let items = (0..20).map(|i| item(&format!("ROW-{i:02}"))).collect();
    let mut picker = ResumePicker::new(items, SessionListScope::Local);
    let (lines, _) = picker.render(100, 18);
    let rows = content_rows(&lines);
    let sessions = rows.iter().position(|r| r == "▸ Sessions").unwrap();
    let footer = rows.iter().position(|r| r.starts_with("  Tab:")).unwrap();
    assert!(
        rows[sessions + 1..footer].iter().all(|r| !r.is_empty()),
        "{rows:?}"
    );
    assert!(rows.iter().any(|r| r.contains("ROW-01")), "{rows:?}");

    // Very short: the header's blank rows go too, and the rows still hit-test.
    let (lines, width) = picker.render(100, 12);
    let rows = content_rows(&lines);
    assert_eq!(
        rows[1], "  Scope:   [Local Folder]   All Folders",
        "{rows:?}"
    );
    assert_eq!(rows[2], "  Search:", "{rows:?}");
    assert_eq!(rows[3], "▸ Sessions", "{rows:?}");
    assert!(rows[4].contains("ROW-00"), "{rows:?}");
    let x = (100 - width) / 2 + 3;
    let y = (12 - lines.len()) / 2 + 1 + 4;
    assert_eq!(
        picker.handle_input(&Key::MousePress(x as u16, y as u16)),
        ResumePickerEvent::Selected("ROW-00".into())
    );
    let y = (12 - lines.len()) / 2 + 1 + 2;
    picker.handle_input(&Key::MousePress(x as u16, y as u16));
    let (lines, _) = picker.render(100, 12);
    assert_eq!(content_rows(&lines)[2], "▸ Search:  ▏");
}

#[test]
fn mouse_rows_track_the_rendered_layout() {
    let mut picker = ResumePicker::new(vec![item("a"), item("b")], SessionListScope::Local);
    let (lines, width) = picker.render(100, 30);
    let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
    let left = (100 - width) / 2;
    let top = (30 - lines.len()) / 2;
    let row = |needle: &str| top + plain.iter().position(|l| l.contains(needle)).unwrap();
    let press = |p: &mut ResumePicker, x: usize, y: usize| {
        p.handle_input(&Key::MousePress(x as u16, y as u16))
    };
    assert_eq!(
        press(&mut picker, left + 3, row("Search:")),
        ResumePickerEvent::Pending
    );
    let (lines, _) = picker.render(100, 30);
    assert!(strip_ansi(&lines[row("Search:") - top]).contains("▸ Search:"));
    assert_eq!(
        press(&mut picker, left + 3, row("Sessions")),
        ResumePickerEvent::Pending
    );
    let (lines, _) = picker.render(100, 30);
    assert!(strip_ansi(&lines[row("Sessions") - top]).contains("▸ Sessions"));
    assert_eq!(
        press(&mut picker, left + 3, row("Sessions") + 2),
        ResumePickerEvent::Selected("b".into())
    );
    // The blank row above the scope switch and the one under Search do nothing.
    assert_eq!(
        press(&mut picker, left + 3, row("Resume session") + 1),
        ResumePickerEvent::Pending
    );
    assert_eq!(
        press(&mut picker, left + 3, row("Search:") + 1),
        ResumePickerEvent::Pending
    );
    assert_eq!(picker.selected_item().unwrap().value, "b");
}
