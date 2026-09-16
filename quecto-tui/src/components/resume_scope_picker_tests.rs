//! Unit tests for scope-aware resume picker (#2001 D5).

use super::*;

fn row(
    key: &str,
    title: &str,
    legacy: bool,
    cross: bool,
    missing: bool,
) -> ResumePickerRow {
    ResumePickerRow {
        key: key.into(),
        title: title.into(),
        repository_label: Some("repo".into()),
        execution_path: Some("/path".into()),
        is_legacy_unscoped: legacy,
        cross_folder: cross,
        home_missing: missing,
    }
}

fn sample_rows() -> Vec<ResumePickerRow> {
    vec![
        row("chat-local", "local work", false, false, false),
        row("chat-cross", "other folder", false, true, false),
        row("chat-miss", "missing home", false, false, true),
        row("cli:legacy", "old session", true, false, false),
    ]
}

#[test]
fn opens_in_local_scope_by_default() {
    let p = ResumeScopePicker::open_local(sample_rows());
    assert_eq!(p.scope_mode(), PickerScopeMode::Local);
    let (local, global, local_on) = p.scope_control_labels();
    assert_eq!(local, "Local");
    assert_eq!(global, "Global");
    assert!(local_on);
    // Local excludes legacy unscoped.
    assert!(p.visible_rows().iter().all(|r| !r.is_legacy_unscoped));
    assert_eq!(p.visible_rows().len(), 3);
}

#[test]
fn tab_moves_focus_among_control_query_results() {
    let mut p = ResumeScopePicker::open_local(sample_rows());
    assert_eq!(p.focus(), PickerFocus::Results);
    p.handle_key(&Key::Tab);
    assert_eq!(p.focus(), PickerFocus::ScopeControl);
    p.handle_key(&Key::Tab);
    assert_eq!(p.focus(), PickerFocus::Query);
    p.handle_key(&Key::Tab);
    assert_eq!(p.focus(), PickerFocus::Results);
    p.handle_key(&Key::BackTab);
    assert_eq!(p.focus(), PickerFocus::Query);
}

#[test]
fn space_or_enter_on_scope_control_toggles_local_global() {
    let mut p = ResumeScopePicker::open_local(sample_rows());
    p.handle_key(&Key::Tab); // Results -> ScopeControl
    assert_eq!(p.focus(), PickerFocus::ScopeControl);
    p.handle_key(&Key::Char(' '));
    assert_eq!(p.scope_mode(), PickerScopeMode::Global);
    // Global includes legacy.
    assert!(
        p.visible_rows()
            .iter()
            .any(|r| r.key == "cli:legacy")
    );
    p.handle_key(&Key::Enter);
    assert_eq!(p.scope_mode(), PickerScopeMode::Local);
}

#[test]
fn mouse_click_scope_selects_global_or_local() {
    let mut p = ResumeScopePicker::open_local(sample_rows());
    p.mouse_click_scope(true);
    assert_eq!(p.scope_mode(), PickerScopeMode::Global);
    assert_eq!(p.focus(), PickerFocus::ScopeControl);
    p.mouse_click_scope(false);
    assert_eq!(p.scope_mode(), PickerScopeMode::Local);
}

#[test]
fn global_query_filters_metadata_title_key_repo_path() {
    let mut p = ResumeScopePicker::open_local(sample_rows());
    p.mouse_click_scope(true);
    p.handle_key(&Key::Tab); // ScopeControl -> Query
    assert_eq!(p.focus(), PickerFocus::Query);
    for c in "legacy".chars() {
        p.handle_key(&Key::Char(c));
    }
    assert_eq!(p.visible_rows().len(), 1);
    assert_eq!(p.visible_rows()[0].key, "cli:legacy");
}

#[test]
fn same_scope_enter_resumes_key() {
    let mut p = ResumeScopePicker::open_local(sample_rows());
    // selected 0 = chat-local
    let action = p.handle_key(&Key::Enter);
    assert_eq!(
        action,
        PickerAction::ResumeSameScope {
            key: "chat-local".into()
        }
    );
}

#[test]
fn cross_folder_row_opens_open_fork_cancel_dialog() {
    let mut p = ResumeScopePicker::open_local(sample_rows());
    p.handle_key(&Key::Down); // chat-cross
    let action = p.handle_key(&Key::Enter);
    assert_eq!(action, PickerAction::Pending);
    assert!(p.dialog_open());
    assert_eq!(p.dialog_kind(), Some(DispositionDialogKind::CrossFolder));
    let choices = p.dialog_choices().unwrap();
    assert_eq!(choices[0], ResumeDispositionUi::OpenOriginal);
    assert_eq!(choices[1], ResumeDispositionUi::ForkCurrent);
    assert_eq!(choices[2], ResumeDispositionUi::Cancel);
    // Confirm Open original
    let action = p.handle_key(&Key::Enter);
    assert_eq!(
        action,
        PickerAction::Disposition {
            key: "chat-cross".into(),
            disposition: ResumeDispositionUi::OpenOriginal,
        }
    );
}

#[test]
fn missing_home_row_opens_locate_fork_cancel_dialog() {
    let mut p = ResumeScopePicker::open_local(sample_rows());
    p.handle_key(&Key::Down);
    p.handle_key(&Key::Down); // chat-miss
    p.handle_key(&Key::Enter);
    assert_eq!(p.dialog_kind(), Some(DispositionDialogKind::MissingHome));
    let choices = p.dialog_choices().unwrap();
    assert_eq!(choices[0], ResumeDispositionUi::Locate);
    // Select Fork
    p.handle_key(&Key::Down);
    let action = p.handle_key(&Key::Enter);
    assert_eq!(
        action,
        PickerAction::Disposition {
            key: "chat-miss".into(),
            disposition: ResumeDispositionUi::ForkCurrent,
        }
    );
}

#[test]
fn escape_cancels_dialog_then_picker() {
    let mut p = ResumeScopePicker::open_local(sample_rows());
    p.handle_key(&Key::Down);
    p.handle_key(&Key::Enter); // open dialog
    assert!(p.dialog_open());
    assert_eq!(p.handle_key(&Key::Escape), PickerAction::Pending);
    assert!(!p.dialog_open());
    assert_eq!(p.handle_key(&Key::Escape), PickerAction::Dismissed);
}

#[test]
fn mouse_click_row_activates_same_as_enter() {
    let mut p = ResumeScopePicker::open_local(sample_rows());
    let action = p.mouse_click_row(0);
    assert_eq!(
        action,
        PickerAction::ResumeSameScope {
            key: "chat-local".into()
        }
    );
}

#[test]
fn scope_control_line_marks_local_selection() {
    let p = ResumeScopePicker::open_local(sample_rows());
    let line = p.render_scope_control_line();
    assert!(line.contains("[Local]"));
    assert!(line.contains("Global"));
}

/// Characterization: Ctrl+G is jump-to-latest chat, not a resume binding.
/// This picker must not claim Ctrl+G; shell continues to route Ctrl+G to chat.
#[test]
fn picker_does_not_consume_ctrl_g_as_resume_action() {
    let mut p = ResumeScopePicker::open_local(sample_rows());
    let action = p.handle_key(&Key::Ctrl('g'));
    assert_eq!(action, PickerAction::Pending);
    // Picker state unchanged (still open, local).
    assert_eq!(p.scope_mode(), PickerScopeMode::Local);
}
