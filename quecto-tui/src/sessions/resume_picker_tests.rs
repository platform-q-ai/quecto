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
fn scope_focus_and_query_cycle_with_title_only_filter() {
    let mut picker = ResumePicker::new(vec![item("a"), item("b")], SessionListScope::Local);
    picker.handle_input(&Key::Tab);
    assert_eq!(
        picker.handle_input(&Key::Char(' ')),
        ResumePickerEvent::ScopeChanged(SessionListScope::Global)
    );
    picker.sync_items(vec![item("a"), item("b")]);
    picker.handle_input(&Key::Tab);
    picker.handle_input(&Key::Char('b'));
    assert_eq!(picker.selected_item().unwrap().value, "b");
    picker.handle_input(&Key::Tab);
    assert_eq!(
        picker.handle_input(&Key::Enter),
        ResumePickerEvent::Selected("b".into())
    );
    picker.handle_input(&Key::BackTab);
    picker.handle_input(&Key::Backspace);
    picker.handle_input(&Key::Char('/'));
    assert!(picker.selected_item().is_none());
    assert_eq!(
        picker.handle_input(&Key::Escape),
        ResumePickerEvent::Dismissed
    );
}
#[test]
fn empty_picker_scope_mouse_is_centered_and_visible() {
    let mut picker = ResumePicker::default();
    let (lines, width) = picker.render(100, 30);
    assert!(
        lines
            .iter()
            .any(|s| s.contains("Local") && s.contains("Global"))
    );
    let x = (100 - width) / 2 + 2 + 10;
    let y = (30 - lines.len()) / 2 + 2;
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
    let y = (40 - lines.len()) / 2 + 4;
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
    let y = (40 - lines.len()) / 2 + 5; // details, immediately after the sole result
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
    assert!(text.contains("Esc cancel"));
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
                .position(|line| line.contains("Esc cancel"))
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
