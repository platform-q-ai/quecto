//! Review round 3 at the picker (#2010): the owed-Enter cue survives any
//! width the header renders at (R3-T4), and whitespace in the box — pasted or
//! typed — is a word break, never deleted (R3-T5).
use super::resume_picker::{ResumePicker, ResumePickerEvent, RowsState};
use crate::components::select_list::SelectItem;
use crate::protocol::session_payloads::SessionListScope;
use crate::shell::keys::Key;

fn picker() -> ResumePicker {
    let item = SelectItem {
        value: "session:one".into(),
        label: "title of one".into(),
        description: Some("/work/one · Resume · ID one".into()),
    };
    ResumePicker::new(vec![item], SessionListScope::Local)
}

/// `z`, back to the rows, and — when `owed` — the Enter typed ahead.
fn searching(owed: bool) -> ResumePicker {
    let mut picker = picker();
    picker.handle_input(&Key::BackTab);
    picker.handle_input(&Key::Char('z'));
    picker.handle_input(&Key::Enter);
    if owed {
        picker.handle_input(&Key::Enter);
    }
    picker
}

fn header(picker: &mut ResumePicker, width: usize) -> Option<String> {
    let (lines, _) = picker.render(width, 24);
    let lines = crate::components::ansi::strip_ansi(&lines.join("\n"));
    let line = lines.lines().find(|line| line.contains("▸ "))?;
    let text = line[line.find("▸ ").unwrap()..].trim_start_matches("▸ ");
    let text = text.trim_end_matches(['│', ' ']).to_string();
    (!text.is_empty()).then_some(text)
}

#[test]
fn an_owed_enter_is_on_screen_at_every_width_the_header_renders_at() {
    let mut rendered = 0;
    for width in 1..=130 {
        let Some(plain) = header(&mut searching(false), width) else {
            continue;
        };
        rendered += 1;
        let owed = header(&mut searching(true), width).unwrap_or_default();
        assert_ne!(owed, plain, "width {width}");
        assert!(owed.contains('⏎'), "width {width}: {owed}");
    }
    assert!(rendered > 100, "the sweep saw the header: {rendered}");
    // The three forms, widest first.
    let owed = |width| header(&mut searching(true), width).unwrap();
    assert_eq!(owed(120), "Sessions · Searching… ⏎ will open the top match");
    assert_eq!(owed(40), "Sessions · ⏎ Searching…");
    assert!(owed(24).starts_with("⏎ Sess"), "{}", owed(24));
}

#[test]
fn pasted_whitespace_is_a_word_break_never_deleted() {
    let mut pasted = picker();
    assert_eq!(
        pasted.handle_input(&Key::Paste(
            "\u{a0}foo\tbar\u{a0}baz\u{2028}qux \u{3000}\u{200b} x\t".into()
        )),
        ResumePickerEvent::QueryChanged("foo bar baz qux x".into())
    );
    // Typed (a no-break space from the keyboard): a space too.
    let mut typed = picker();
    typed.handle_input(&Key::BackTab);
    for ch in "fix\u{a0}bug".chars() {
        typed.handle_input(&Key::Char(ch));
    }
    assert_eq!(typed.query(), "fix bug");
    // The 256 bound is of the text as it lands in the box.
    let mut full = picker();
    let text = format!("{}\t\t{}", "a".repeat(127), "b".repeat(128));
    full.handle_input(&Key::Paste(text));
    assert_eq!(full.query().chars().count(), 256);
    assert_eq!(full.rows_state(), RowsState::Searching);
}
