//! R1-T1 / R1-T8 / R1-T2 / R1-T11 at the picker: Enter and the mouse act only
//! on rows that are the settled answer for the text in the box; an Enter
//! typed ahead of the answer is deferred to its top-ranked row; the cursor
//! follows the ranking unless the user placed it; the states a search can be
//! in are on screen; and a key can be pasted.
use super::resume_picker::{ResumePicker, ResumePickerEvent, RowsState};
use crate::components::select_list::SelectItem;
use crate::protocol::session_payloads::SessionListScope;
use crate::shell::keys::Key;

fn item(id: &str) -> SelectItem {
    SelectItem {
        value: format!("session:{id}"),
        label: format!("title of {id}"),
        description: Some(format!("/work/{id} · Resume · ID {id}")),
    }
}

fn listed() -> ResumePicker {
    let rows = vec![item("listed-1"), item("listed-2"), item("zebra")];
    ResumePicker::new(rows, SessionListScope::Local)
}

/// Focus the search box, type `text`, and return to the results with Enter —
/// the reviewer's `query ⏎` — leaving the picker unsettled.
fn type_ahead(picker: &mut ResumePicker, text: &str) {
    picker.handle_input(&Key::BackTab);
    for ch in text.chars() {
        let typed = picker.handle_input(&Key::Char(ch));
        assert!(matches!(typed, ResumePickerEvent::QueryChanged(_)));
    }
    assert_eq!(
        picker.rows_state(),
        RowsState::Searching,
        "typing unsettles"
    );
    assert_eq!(picker.handle_input(&Key::Enter), ResumePickerEvent::Pending);
}

fn frame(picker: &mut ResumePicker) -> String {
    let (lines, _) = picker.render(90, 30);
    crate::components::ansi::strip_ansi(&lines.join("\n"))
}

#[test]
fn enter_typed_ahead_of_the_answer_resumes_the_answers_top_row_never_a_listed_one() {
    let mut picker = listed();
    type_ahead(&mut picker, "zebra");
    // The second Enter lands on the still-listed rows: nothing is selected.
    assert_eq!(picker.handle_input(&Key::Enter), ResumePickerEvent::Pending);
    assert_eq!(picker.selected_item().unwrap().value, "session:listed-1");
    // An overtaken answer is progress, not the answer: still nothing.
    picker.sync_items(vec![item("zeb-1"), item("zebra")]);
    assert_eq!(picker.rows_state(), RowsState::Searching);
    // The settled answer for the text in the box: its top-ranked row.
    picker.sync_items(vec![item("zebra"), item("zebra-2")]);
    let resumed = picker.set_rows_state(RowsState::Settled);
    assert_eq!(resumed.as_deref(), Some("session:zebra"));
    assert_eq!(
        picker.set_rows_state(RowsState::Settled),
        None,
        "acted on once"
    );
}

#[test]
fn a_deferred_enter_resumes_nothing_when_nothing_matches() {
    let mut picker = listed();
    type_ahead(&mut picker, "zzzz");
    assert_eq!(picker.handle_input(&Key::Enter), ResumePickerEvent::Pending);
    picker.sync_items(Vec::new());
    assert_eq!(picker.set_rows_state(RowsState::Settled), None);
    // And a later answer with rows is not resumed by that old Enter.
    picker.sync_items(vec![item("zebra")]);
    assert_eq!(picker.set_rows_state(RowsState::Settled), None);
}

#[test]
fn moving_the_cursor_while_unsettled_cancels_the_deferred_enter_and_any_later_one() {
    let mut picker = listed();
    type_ahead(&mut picker, "zebra");
    picker.handle_input(&Key::Enter);
    picker.handle_input(&Key::Down);
    picker.sync_items(vec![item("zebra")]);
    assert_eq!(
        picker.set_rows_state(RowsState::Settled),
        None,
        "moved after Enter"
    );
    // Moved first, Enter second: the user is pointing at a stale row.
    let mut picker = listed();
    type_ahead(&mut picker, "zebra");
    picker.handle_input(&Key::Down);
    assert_eq!(picker.handle_input(&Key::Enter), ResumePickerEvent::Pending);
    picker.sync_items(vec![item("zebra")]);
    assert_eq!(picker.set_rows_state(RowsState::Settled), None);
    // Settled, Enter acts at once, on the row under the cursor.
    assert_eq!(
        picker.handle_input(&Key::Enter),
        ResumePickerEvent::Selected("session:zebra".into())
    );
}

#[test]
fn typing_a_scope_change_and_escape_each_clear_a_deferred_enter() {
    let mut picker = listed();
    type_ahead(&mut picker, "zeb");
    picker.handle_input(&Key::Enter);
    picker.handle_input(&Key::BackTab);
    picker.handle_input(&Key::Char('r'));
    picker.sync_items(vec![item("zebra")]);
    assert_eq!(picker.set_rows_state(RowsState::Settled), None, "typed on");

    let mut picker = listed();
    type_ahead(&mut picker, "zeb");
    picker.handle_input(&Key::Enter);
    picker.handle_input(&Key::Tab);
    assert_eq!(
        picker.handle_input(&Key::Char(' ')),
        ResumePickerEvent::ScopeChanged(SessionListScope::Global)
    );
    picker.sync_items(vec![item("zebra")]);
    assert_eq!(
        picker.set_rows_state(RowsState::Settled),
        None,
        "scope changed"
    );

    let mut picker = listed();
    type_ahead(&mut picker, "zeb");
    picker.handle_input(&Key::Enter);
    assert_eq!(
        picker.handle_input(&Key::Escape),
        ResumePickerEvent::Dismissed
    );
    picker.sync_items(vec![item("zebra")]);
    assert_eq!(picker.set_rows_state(RowsState::Settled), None, "escaped");
}

#[test]
fn a_click_on_an_unsettled_row_moves_the_cursor_and_selects_nothing() {
    let mut picker = listed();
    let _ = picker.render(90, 30);
    let (lines, width) = picker.render(90, 30);
    let top = (30 - lines.len().min(26)) / 2;
    let left = (90 - width) / 2;
    // Second result row: border + six header rows + one.
    let (x, y) = ((left + 8) as u16, (top + 1 + 6 + 1) as u16);
    type_ahead(&mut picker, "zebra");
    assert_eq!(
        picker.handle_input(&Key::MousePress(x, y)),
        ResumePickerEvent::Pending
    );
    assert_eq!(picker.selected_item().unwrap().value, "session:listed-2");
    picker.sync_items(vec![item("listed-2"), item("zebra")]);
    assert_eq!(picker.set_rows_state(RowsState::Settled), None);
    // Settled: the same click resumes the row it lands on.
    assert_eq!(
        picker.handle_input(&Key::MousePress(x, y)),
        ResumePickerEvent::Selected("session:zebra".into())
    );
}

#[test]
fn stalled_rows_are_never_acted_on_and_say_why() {
    let mut picker = listed();
    type_ahead(&mut picker, "zebra");
    assert_eq!(picker.set_rows_state(RowsState::Stalled), None);
    assert_eq!(picker.handle_input(&Key::Enter), ResumePickerEvent::Pending);
    assert_eq!(
        picker.set_rows_state(RowsState::Settled),
        None,
        "no Enter was deferred"
    );
}

/// R1-T8: an answer is ranked, so the cursor goes to its best row — unless
/// the user placed the cursor and that session is still there.
#[test]
fn replaced_rows_put_the_cursor_on_the_top_row_unless_the_user_placed_it() {
    let mut picker = listed();
    // Untouched cursor, its key still present lower down: top row wins.
    picker.sync_items(vec![item("zebra"), item("listed-1")]);
    assert_eq!(picker.selected_item().unwrap().value, "session:zebra");
    // Placed by the user, still present: kept, wherever it moved to.
    picker.handle_input(&Key::Down);
    assert_eq!(picker.selected_item().unwrap().value, "session:listed-1");
    picker.sync_items(vec![item("a"), item("b"), item("listed-1")]);
    assert_eq!(picker.selected_item().unwrap().value, "session:listed-1");
    // Placed, but gone from the answer: the top row, not the old index.
    picker.sync_items(vec![item("x"), item("y"), item("z")]);
    assert_eq!(picker.selected_item().unwrap().value, "session:x");
    // A new text is a new question: the old placement no longer counts.
    picker.handle_input(&Key::Down);
    picker.handle_input(&Key::BackTab);
    picker.handle_input(&Key::Char('q'));
    picker.sync_items(vec![item("z"), item("y")]);
    assert_eq!(picker.selected_item().unwrap().value, "session:z");
}

/// R1-T2 / R1-T3: what the search is doing is on screen.
#[test]
fn searching_a_notice_and_an_empty_answer_are_rendered() {
    let mut picker = listed();
    assert!(!frame(&mut picker).contains("Searching…"));
    type_ahead(&mut picker, "mess");
    let shown = frame(&mut picker);
    assert!(shown.contains("Sessions · Searching…"), "{shown}");
    picker.sync_items(vec![item("a"), item("b")]);
    picker.set_notice(Some("Showing 2 of 5,200 — keep typing to narrow".into()));
    picker.set_rows_state(RowsState::Settled);
    let shown = frame(&mut picker);
    assert!(
        shown.contains("Showing 2 of 5,200 — keep typing to narrow"),
        "{shown}"
    );
    assert!(!shown.contains("Searching…"), "{shown}");
    // No rows: the notice stands where the rows would, never "No items".
    picker.sync_items(Vec::new());
    picker.set_notice(Some("No sessions match \"mess\" in Local Folder".into()));
    let shown = frame(&mut picker);
    assert!(
        shown.contains("No sessions match \"mess\" in Local Folder"),
        "{shown}"
    );
    assert!(!shown.contains("No items"), "{shown}");
    // Unanswered and nothing asking.
    picker.set_rows_state(RowsState::Stalled);
    assert!(frame(&mut picker).contains("Sessions · Search did not answer"));
    // Every state fits a short, narrow terminal without panicking.
    for (width, height) in [(20, 8), (10, 5), (40, 12), (120, 40)] {
        let _ = picker.render(width, height);
    }
}

/// R1-T11: a key is pasted, not typed.
#[test]
fn a_paste_into_the_search_box_is_sanitised_single_line_and_bounded() {
    let mut picker = listed();
    // Ignored outside the search box.
    assert_eq!(
        picker.handle_input(&Key::Paste("x".into())),
        ResumePickerEvent::Pending
    );
    picker.handle_input(&Key::BackTab);
    assert_eq!(
        picker.handle_input(&Key::Paste(
            "  chat-1776-ab\u{1b}[31m\u{202e}cd\r\nsecond line".into()
        )),
        ResumePickerEvent::QueryChanged("chat-1776-ab[31mcd".into())
    );
    assert_eq!(picker.rows_state(), RowsState::Searching);
    // The box holds 256 characters — room for any key.
    let mut picker = listed();
    picker.handle_input(&Key::BackTab);
    let long_key = "k".repeat(256);
    assert_eq!(
        picker.handle_input(&Key::Paste(long_key.clone())),
        ResumePickerEvent::QueryChanged(long_key.clone())
    );
    assert_eq!(
        picker.handle_input(&Key::Char('x')),
        ResumePickerEvent::Pending,
        "full"
    );
    // Longer than the box: refused whole — a prefix of a key names nothing.
    let mut picker = listed();
    picker.handle_input(&Key::BackTab);
    picker.handle_input(&Key::Char('a'));
    assert_eq!(
        picker.handle_input(&Key::Paste("k".repeat(256))),
        ResumePickerEvent::Pending
    );
    assert_eq!(picker.query(), "a");
    let shown = frame(&mut picker);
    assert!(
        shown.contains("Paste refused: longer than 256 characters"),
        "{shown}"
    );
    // Whitespace-only and empty pastes change nothing.
    assert_eq!(
        picker.handle_input(&Key::Paste(" \n".into())),
        ResumePickerEvent::Pending
    );
}
