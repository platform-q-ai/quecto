//! The notice a refused resume opens (#2045): what it says, that it is whole on
//! real terminal sizes, that hostile text cannot shape it, and that it has no
//! action — any of its keys only closes it.
use super::*;
use crate::components::utils::visible_width;

const FOLDER: &str = "/home/user/Documents/github/some-organisation/a-rather-long-project-name/\
    packages/frontend-application";

fn refusal(code: ResumeRefusalCode, kind: &str) -> ResumeRefusal {
    ResumeRefusal {
        code,
        session_key: Some("cli:foreign".to_string()),
        kind: kind.to_string(),
        execution_path: Some(FOLDER.to_string()),
        detail: None,
        command: Some(format!("cd '{FOLDER}' && quecto-tui")),
        resume: Some("/resume cli:foreign".to_string()),
    }
}

fn elsewhere() -> ResumeRefusal {
    refusal(ResumeRefusalCode::BelongsElsewhere, "cross_folder")
}

fn lines(dialog: &mut ResumeDecisionDialog, width: usize, height: usize) -> Vec<String> {
    let (lines, panel) = dialog.render_overlay(width, height);
    let lines: Vec<String> = lines
        .iter()
        .map(|line| crate::components::ansi::strip_ansi(line))
        .collect();
    assert!(lines.len() <= height.saturating_sub(4).max(1), "{lines:#?}");
    for line in &lines {
        assert!(visible_width(line) <= panel.max(1), "{line:?} in {panel}");
    }
    lines
}

/// The characters between the box borders, line breaks removed: a command
/// wrapped at the column reads back exactly.
fn unwrapped(lines: &[String]) -> String {
    lines
        .iter()
        .map(|l| l.trim_matches(['│', ' ']))
        .collect::<Vec<_>>()
        .join("")
}

#[test]
fn every_kind_says_what_is_wrong_in_plain_words() {
    for (code, kind, title) in [
        (
            ResumeRefusalCode::BelongsElsewhere,
            "cross_folder",
            "This session belongs to another folder",
        ),
        (
            ResumeRefusalCode::HomeMissing,
            "home_missing",
            "This session's folder is missing or unreadable",
        ),
        (
            ResumeRefusalCode::HomeChanged,
            "home_changed",
            "This session's folder has changed",
        ),
        (
            ResumeRefusalCode::HomeUnknown,
            "home_unknown",
            "This session's folder can't be read",
        ),
        (
            ResumeRefusalCode::NoHomeRecorded,
            "legacy_unscoped",
            "No folder is recorded for this session",
        ),
    ] {
        let mut dialog = ResumeDecisionDialog::new(refusal(code, kind), None);
        let shown = lines(&mut dialog, 100, 30).join("\n");
        assert!(shown.contains(title), "{kind}: {shown}");
        assert!(!shown.contains(kind), "no wire jargon: {shown}");
    }
}

#[test]
fn the_command_and_the_resume_step_are_whole_on_real_terminals() {
    let command = format!("cd '{FOLDER}' && quecto-tui");
    for (width, height) in [(120, 40), (80, 24), (40, 20)] {
        let mut dialog = ResumeDecisionDialog::new(elsewhere(), Some("Fix the renderer"));
        let shown = lines(&mut dialog, width, height);
        let text = unwrapped(&shown);
        assert!(
            text.contains(&command.replace(' ', "")) || text.contains(&command),
            "{width}x{height}: the command, never elided: {shown:#?}"
        );
        assert!(!text.contains('…') || !command.contains('…'), "{shown:#?}");
        assert!(text.contains("/resumecli:foreign") || text.contains("/resume cli:foreign"));
        assert!(shown.iter().any(|l| l.contains("Enter or Esc to close")));
        let bottom = shown.iter().position(|l| l.contains('└')).expect("border");
        let footer = shown
            .iter()
            .position(|l| l.contains("Enter or Esc to close"))
            .expect("footer");
        assert!(footer < bottom, "{width}x{height}: {shown:#?}");
    }
}

#[test]
fn the_picked_title_is_shown_when_there_is_one() {
    let mut with = ResumeDecisionDialog::new(elsewhere(), Some("Fix the renderer"));
    assert!(
        lines(&mut with, 80, 24)
            .join("\n")
            .contains("Fix the renderer")
    );
    let mut without = ResumeDecisionDialog::new(elsewhere(), None);
    assert!(
        !lines(&mut without, 80, 24)
            .join("\n")
            .contains("Fix the renderer")
    );
}

#[test]
fn a_command_the_sanitiser_would_change_is_not_shown_at_all() {
    // A harness that spelled a command with a control or a reordering
    // character is not believed: a changed command runs somewhere else.
    for hostile in [
        "cd '/w/a\u{1b}[2Jb' && quecto-tui",
        "cd '/w/rtl\u{202e}gnp' && quecto-tui",
        "cd '/w/zero\u{200b}width' && quecto-tui",
        "cd '/w/two\nlines' && quecto-tui",
    ] {
        let mut refused = elsewhere();
        refused.command = Some(hostile.to_string());
        let mut dialog = ResumeDecisionDialog::new(refused, None);
        let shown = lines(&mut dialog, 100, 30).join("\n");
        assert!(!shown.contains("quecto-tui"), "{hostile:?}: {shown}");
        assert!(
            shown.contains("Open quecto in that folder, then type:"),
            "{shown}"
        );
        assert!(shown.contains("/resume cli:foreign"), "{shown}");
        assert!(
            !shown.contains('\u{1b}') && !shown.contains('\u{202e}'),
            "{shown:?}"
        );
    }
}

#[test]
fn hostile_folder_and_detail_cannot_shape_the_panel() {
    let mut refused = refusal(ResumeRefusalCode::HomeMissing, "home_missing");
    refused.execution_path = Some("/w/\u{1b}]0;owned\u{7}\u{202e}evil\nSECOND LINE".to_string());
    refused.detail = Some("Permission denied\u{1b}[31m (os error 13)".to_string());
    refused.command = None;
    let mut dialog = ResumeDecisionDialog::new(refused, Some("title\u{1b}[2J\u{202e}"));
    for (width, height) in [(80, 24), (40, 20), (20, 8), (1, 1)] {
        let shown = lines(&mut dialog, width, height).join("\n");
        for bad in ['\u{1b}', '\u{7}', '\u{202e}'] {
            assert!(
                !shown.contains(bad),
                "{width}x{height}: {bad:?} in {shown:?}"
            );
        }
    }
    let shown = lines(&mut dialog, 80, 24).join("\n");
    assert!(shown.contains("Permission denied"), "{shown}");
}

#[test]
fn a_short_panel_keeps_how_to_open_it_and_drops_the_explanation_first() {
    let mut refused = elsewhere();
    refused.detail = Some("some detail from the harness".to_string());
    let mut dialog = ResumeDecisionDialog::new(refused, Some("A picked title"));
    let shown = lines(&mut dialog, 120, 14);
    let text = unwrapped(&shown);
    assert!(
        text.contains("&&quecto-tui") || text.contains("&& quecto-tui"),
        "{shown:#?}"
    );
    assert!(text.contains("/resume cli:foreign"), "{shown:#?}");
    assert!(text.contains("Enter or Esc to close"), "{shown:#?}");
    assert!(!text.contains("some detail from the harness"), "{shown:#?}");
}

/// #2056 review: a command that does not fit is not shown in part. On every
/// terminal size, for a short and for a very long folder, the panel either
/// shows the command character for character or does not show one at all —
/// and it always says what to type, and always keeps its footer.
#[test]
fn a_command_is_shown_whole_or_not_at_all_on_every_size() {
    let long = format!("/srv/{}", "a-very-long-directory-name/".repeat(24));
    for folder in [FOLDER.to_string(), long] {
        let command = format!("cd '{folder}' && quecto-tui");
        for width in [24, 40, 60, 80, 120] {
            for height in [10, 12, 16, 20, 24, 40] {
                let mut refused = elsewhere();
                refused.execution_path = Some(folder.clone());
                refused.command = Some(command.clone());
                let mut dialog = ResumeDecisionDialog::new(refused, Some("A picked title"));
                let shown = lines(&mut dialog, width, height);
                let text = unwrapped(&shown);
                let at = format!(
                    "{width}x{height}, folder of {} chars: {shown:#?}",
                    folder.len()
                );
                if text.contains("cd '") {
                    assert!(text.contains(&command), "a partial command at {at}");
                    assert!(text.contains("then type:"), "{at}");
                }
                // The footer is never clipped: the panel picks a form that fits.
                assert!(
                    shown.iter().any(|line| {
                        let inside = line.trim_matches(['│', ' ']);
                        [FOOTER, FOOTER_NARROW, "Esc"].contains(&inside)
                    }),
                    "the footer is lost at {at}"
                );
                if height >= 16 && width >= 40 {
                    assert!(
                        text.contains("/resume cli:foreign"),
                        "the step is lost at {at}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_required_small_terminal_falls_back_rather_than_cutting_a_long_command() {
    let folder = format!("/srv/{}", "a-very-long-directory-name/".repeat(24));
    let mut refused = elsewhere();
    refused.execution_path = Some(folder.clone());
    refused.command = Some(format!("cd '{folder}' && quecto-tui"));
    let mut dialog = ResumeDecisionDialog::new(refused, None);
    let text = unwrapped(&lines(&mut dialog, 40, 20));
    assert!(!text.contains("cd '"), "{text}");
    // The label wraps on words at this width; nothing in it is clipped.
    assert!(
        text.replace(' ', "")
            .contains("Openquectointhatfolder,thentype:"),
        "{text}"
    );
    assert!(
        text.contains("/resume cli:foreign") && text.contains(FOOTER),
        "{text}"
    );
}

/// #2056 review: going to the recorded folder resumes ONE kind. The others say
/// why the session can't be resumed — and never show a command or a step,
/// even if a harness sent them.
#[test]
fn only_a_session_that_lives_elsewhere_is_told_to_go_there() {
    for (code, kind, says) in [
        (
            ResumeRefusalCode::HomeMissing,
            "home_missing",
            "Bring that folder back",
        ),
        (
            ResumeRefusalCode::HomeChanged,
            "home_changed",
            "a different project now",
        ),
        (
            ResumeRefusalCode::HomeUnknown,
            "home_unknown",
            "folder record can't be read",
        ),
        (
            ResumeRefusalCode::NoHomeRecorded,
            "legacy_unscoped",
            "before quecto tracked folders",
        ),
    ] {
        // `refusal()` carries a command and a resume step: a harness that sent them.
        let mut dialog = ResumeDecisionDialog::new(refusal(code, kind), None);
        let shown = lines(&mut dialog, 100, 30).join(" ");
        let shown = shown.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(shown.contains(says), "{kind}: {shown}");
        for never in ["cd '", "quecto-tui", "/resume", "Open quecto"] {
            assert!(!shown.contains(never), "{kind} shows {never:?}: {shown}");
        }
    }
}

#[test]
fn every_key_it_answers_only_closes_it() {
    let mut dialog = ResumeDecisionDialog::new(elsewhere(), None);
    for key in [Key::Enter, Key::Escape, Key::Ctrl('c')] {
        assert!(dialog.handle_key(&key), "{key:?} closes");
    }
    for key in [Key::Char('y'), Key::Tab, Key::Up, Key::Down] {
        assert!(!dialog.handle_key(&key), "{key:?} does nothing");
    }
}
