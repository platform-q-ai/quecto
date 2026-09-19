//! Review round 2 at the picker (#2010): an Enter is owed only to a SEARCH of
//! typed text, is on screen while owed, and is withdrawn by whatever the user
//! does next (R2-T1, R2-T3); a paste is search text wherever the focus is,
//! one line of it, through the typing filter (R2-T5, R2-T7); and a narrow
//! panel still tells the unsettled states apart (R2-T6).
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
    ResumePicker::new(
        vec![item("listed-1"), item("listed-2")],
        SessionListScope::Local,
    )
}

/// `text ⏎ ⏎`: typed in the box, back to the rows, an Enter owed.
fn owed(text: &str) -> ResumePicker {
    let mut picker = listed();
    picker.handle_input(&Key::BackTab);
    for ch in text.chars() {
        picker.handle_input(&Key::Char(ch));
    }
    picker.handle_input(&Key::Enter);
    picker.handle_input(&Key::Enter);
    picker
}

fn frame(picker: &mut ResumePicker, width: usize) -> String {
    let (lines, _) = picker.render(width, 30);
    crate::components::ansi::strip_ansi(&lines.join("\n"))
}

/// What the settled answer `zebra` makes of the picker's owed Enter.
fn settled(mut picker: ResumePicker) -> Option<String> {
    picker.sync_items(vec![item("zebra")]);
    picker.set_rows_state(RowsState::Settled)
}

#[test]
fn an_enter_is_owed_to_a_search_never_to_a_listing_or_an_empty_box() {
    assert_eq!(settled(owed("zebra")).as_deref(), Some("session:zebra"));
    // Loading: the first listing, a scope re-list, an emptied box.
    let mut picker = listed();
    picker.set_rows_state(RowsState::Loading);
    picker.handle_input(&Key::Enter);
    assert_eq!(settled(picker), None, "a listing is owed nothing");
    let mut picker = owed("z");
    picker.set_rows_state(RowsState::Loading);
    picker.handle_input(&Key::Enter);
    assert_eq!(
        settled(picker),
        None,
        "a re-list under text is owed nothing"
    );
    // Nothing visible in the box is no search text.
    assert_eq!(settled(owed("  ")), None);
    // An emptied box asks for a listing: it says so, and defers nothing.
    let mut picker = owed("z");
    picker.handle_input(&Key::BackTab);
    picker.handle_input(&Key::Backspace);
    assert_eq!(picker.rows_state(), RowsState::Loading);
    assert!(frame(&mut picker, 90).contains("Sessions · Loading…"));
}

#[test]
fn whatever_the_user_does_next_withdraws_the_owed_enter() {
    let (lines, width) = listed().render(90, 30);
    let (left, top) = ((90 - width) / 2, (30 - lines.len().min(26)) / 2);
    let at = |row: usize| Key::MousePress((left + 8) as u16, (top + 1 + row) as u16);
    let cases = [
        ("Tab", Key::Tab),
        ("Shift-Tab", Key::BackTab),
        ("cursor down", Key::Down),
        ("wheel", Key::ScrollDown),
        ("a click on the Search row", at(3)),
        ("a click on the Scope row", at(2)),
        ("a click on the Sessions row", at(5)),
        ("a click on a result", at(7)),
        ("a paste", Key::Paste("x".into())),
        ("a stray key", Key::Home),
    ];
    for (what, key) in cases {
        let mut picker = owed("zebra");
        let _ = picker.render(90, 30);
        assert!(frame(&mut picker, 90).contains("⏎ will open"), "{what}");
        let _ = picker.handle_input(&key);
        assert!(!frame(&mut picker, 90).contains("⏎"), "{what}: cue gone");
        assert_eq!(settled(picker), None, "{what}");
    }
    // The shell's withdrawal (first timeout), and every unsettled state.
    let mut picker = owed("zebra");
    picker.withdraw_enter();
    assert_eq!(settled(picker), None, "withdrawn by the shell");
    for state in [
        RowsState::Stalled,
        RowsState::Disconnected,
        RowsState::Loading,
    ] {
        let mut picker = owed("zebra");
        assert_eq!(picker.set_rows_state(state), None);
        assert_eq!(settled(picker), None, "{state:?}");
    }
    // A repeated Enter owes it again; Escape dismisses and owes nothing.
    let mut picker = owed("zebra");
    picker.handle_input(&Key::Down);
    picker.handle_input(&Key::Up);
    let mut picker = {
        picker.handle_input(&Key::Enter);
        picker
    };
    assert_eq!(
        picker.set_rows_state(RowsState::Settled),
        None,
        "cursor placed"
    );
    let mut picker = owed("zebra");
    assert_eq!(
        picker.handle_input(&Key::Escape),
        ResumePickerEvent::Dismissed
    );
    assert_eq!(settled(picker), None, "escaped");
}

/// The owed Enter opens the SETTLED answer's top row — never a progress row.
#[test]
fn the_owed_enter_opens_the_settled_top_row_not_the_progress_one() {
    let mut picker = owed("zebra");
    picker.sync_items(vec![item("progress-top"), item("zebra")]);
    assert_eq!(picker.set_rows_state(RowsState::Searching), None);
    picker.sync_items(vec![item("zebra"), item("progress-top")]);
    assert_eq!(
        picker.set_rows_state(RowsState::Settled).as_deref(),
        Some("session:zebra")
    );
}

#[test]
fn the_unsettled_states_are_told_apart_on_a_narrow_panel() {
    let header = |picker: &mut ResumePicker, width| {
        let shown = frame(picker, width);
        let line = shown.lines().find(|line| line.contains("Sessions ·"));
        line.unwrap_or_else(|| panic!("{shown}")).trim().to_string()
    };
    for width in [30, 34, 60, 90] {
        let mut seen = std::collections::BTreeSet::new();
        let mut owing = owed("zebra");
        seen.insert(header(&mut owing, width));
        for state in [
            RowsState::Loading,
            RowsState::Searching,
            RowsState::Stalled,
            RowsState::Disconnected,
        ] {
            let mut picker = owed("zebra");
            picker.set_rows_state(state);
            picker.handle_input(&Key::Tab);
            seen.insert(header(&mut picker, width));
        }
        assert_eq!(seen.len(), 5, "{width}: {seen:?}");
    }
    // The advice is the notice line's, which wraps instead of being cut.
    let mut picker = owed("zebra");
    picker.set_rows_state(RowsState::Stalled);
    let said = frame(&mut picker, 50).replace('│', " ");
    let said = said.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        said.contains("Search did not answer — edit the text or change Scope to retry"),
        "{said}"
    );
    picker.set_rows_state(RowsState::Disconnected);
    assert!(frame(&mut picker, 90).contains("Disconnected — once reconnected"));
}

#[test]
fn a_paste_is_one_line_of_search_text_wherever_the_focus_is() {
    // Focus on Sessions (where `/resume` opens): the box takes it, and the focus.
    let mut picker = listed();
    assert_eq!(
        picker.handle_input(&Key::Paste("cli:one\rcli:two\r".into())),
        ResumePickerEvent::QueryChanged("cli:one".into())
    );
    let shown = frame(&mut picker, 90);
    assert!(shown.contains("▸ Search:  cli:one"), "{shown}");
    assert!(shown.contains("Pasted the first line only"), "{shown}");
    // The note rides along with the answer's own notice, until the next edit.
    picker.set_notice(Some("No sessions match".into()));
    let shown = frame(&mut picker, 90);
    assert!(
        shown.contains("No sessions match · Pasted the first line only"),
        "{shown}"
    );
    picker.handle_input(&Key::Char('x'));
    assert!(
        !frame(&mut picker, 90).contains("Pasted"),
        "cleared by an edit"
    );
    // Focus on Scope; `\n`, `\r\n` and leading blank lines; one line: no note.
    for (pasted, query) in [
        ("\n\r\n  key-a  \r\n", "key-a"),
        ("a\nb", "a"),
        ("a\r\nb", "a"),
    ] {
        let mut picker = listed();
        picker.handle_input(&Key::Tab);
        assert_eq!(
            picker.handle_input(&Key::Paste(pasted.into())),
            ResumePickerEvent::QueryChanged(query.into())
        );
        let cut = frame(&mut picker, 90).contains("Pasted the first line only");
        assert_eq!(cut, pasted.contains('b'), "{pasted:?}");
    }
    // A refused paste says so in a line that fits, from any focus.
    let mut picker = listed();
    assert_eq!(
        picker.handle_input(&Key::Paste("k".repeat(257))),
        ResumePickerEvent::Pending
    );
    let shown = frame(&mut picker, 80);
    assert!(
        shown.contains("Paste refused: longer than 256 characters") && shown.contains("▸ Search:"),
        "{shown}"
    );
}

#[test]
fn typing_and_pasting_accept_the_same_characters() {
    let text = "cafe\u{301} — plan 🚀 «x» 日本";
    let hostile = "a\u{1b}\u{202e}\u{200b}\u{feff}\u{ad}\tb\u{a0}c";
    let mut pasted = listed();
    pasted.handle_input(&Key::Paste(format!("{text}{hostile}")));
    assert_eq!(pasted.query(), format!("{text}abc"));
    let mut typed = listed();
    typed.handle_input(&Key::BackTab);
    for ch in format!("{text}{hostile}").chars() {
        typed.handle_input(&Key::Char(ch));
    }
    assert_eq!(typed.query(), pasted.query());
}
