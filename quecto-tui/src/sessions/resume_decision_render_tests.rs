//! The dialog on real terminal sizes (#2011 review R1-T2, R1-T3, R1-T6): at
//! 80×24 everything it exists to show is legible; at 40×20 and below it
//! degrades without clipping its footer or its border.
use super::*;
use crate::components::utils::visible_width;

const ASSOCIATE_REASON: &str = "explicit association of a legacy session with a folder is not \
    available yet; start a new session with `-s <name>` — the old transcript stays in place \
    and visible under All Folders";
const FOLDER: &str = "/home/user/Documents/github/some-organisation/a-rather-long-project-name/\
    packages/frontend-application";

fn legacy() -> ResumeDecisionDialog {
    let mut legacy = decision(
        ResumeDecisionKind::LegacyUnscoped,
        vec![
            offer(ResumeAction::Associate, false, Some(ASSOCIATE_REASON)),
            offer(ResumeAction::Cancel, true, None),
        ],
    );
    legacy.session = "chat-1750000000-ab".into();
    legacy.execution_path = Some(FOLDER.into());
    ResumeDecisionDialog::new(legacy)
}

fn cross() -> ResumeDecisionDialog {
    let mut cross = decision(
        ResumeDecisionKind::CrossFolder,
        vec![
            offer(
                ResumeAction::OpenOriginal,
                false,
                Some("start quecto in that folder"),
            ),
            offer(ResumeAction::ForkCurrent, false, Some("not available yet")),
            offer(ResumeAction::Cancel, true, None),
        ],
    );
    cross.execution_path = Some(FOLDER.into());
    ResumeDecisionDialog::new(cross)
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

/// The box is whole: a top border, a bottom border, the footer inside it.
fn assert_whole(lines: &[String]) {
    assert!(lines.first().unwrap().starts_with('┌'), "{lines:#?}");
    assert!(
        lines.last().unwrap().starts_with('└'),
        "footer/border clipped: {lines:#?}"
    );
    assert!(lines.iter().any(|l| l.contains("Enter")), "{lines:#?}");
    assert!(lines.iter().any(|l| l.contains("cancel")), "{lines:#?}");
}

#[test]
fn at_80_by_24_the_folder_every_mark_and_the_reason_are_legible() {
    let mut dialog = legacy();
    let shown = lines(&mut dialog, 80, 24);
    assert_whole(&shown);
    let text = shown.join("\n");
    assert!(
        text.contains("This session was never linked to a folder"),
        "{text}"
    );
    assert!(text.contains("chat-1750000000-ab"), "{text}");
    // The folder is on its own line(s), whole: both ends, no elision needed.
    let joined: String = shown.iter().map(|l| l.trim_matches(['│', ' '])).collect();
    assert!(joined.contains(FOLDER), "{text}");
    assert!(
        text.contains("Associate with a folder — unavailable"),
        "{text}"
    );
    assert!(text.contains("Unavailable: explicit association"), "{text}");
    assert!(text.contains("nothing is restored or linked"), "{text}");

    let mut dialog = cross();
    let text = lines(&mut dialog, 80, 24).join("\n");
    for row in [
        "Open original folder — unavailable",
        "Fork into current folder — unavailable",
    ] {
        assert!(text.contains(row), "every row carries its mark: {text}");
    }
    assert!(text.contains("→ Open original folder"), "{text}");
}

#[test]
fn words_are_never_broken_when_they_fit_the_line() {
    for (width, height) in [(80, 24), (60, 24), (40, 20)] {
        let mut dialog = legacy();
        let shown = lines(&mut dialog, width, height);
        let reason: Vec<&str> = ASSOCIATE_REASON.split_whitespace().collect();
        for line in shown
            .iter()
            .filter(|l| !l.contains('/') && !l.contains('─'))
        {
            for word in line.trim_matches(['│', ' ']).split_whitespace() {
                let word = word.trim_end_matches('…');
                let whole = word.is_empty()
                    || reason.iter().any(|w| w.starts_with(word))
                    || line.contains("Unavailable:")
                    || !ASSOCIATE_REASON.contains(word);
                assert!(whole, "{width}x{height}: {word:?} in {line:?}");
            }
        }
        // No line ends in the middle of a reason word that continues below.
        let text = shown.join("\n");
        assert!(
            !text.contains("Unavailabl\n") && !text.contains("associ\n"),
            "{text}"
        );
    }
}

#[test]
fn at_40_by_20_it_degrades_without_clipping_the_footer_or_the_border() {
    let mut dialog = legacy();
    let shown = lines(&mut dialog, 40, 20);
    assert_whole(&shown);
    let text = shown.join("\n");
    // The folder keeps BOTH ends on at most two lines, elided in the middle.
    assert!(text.contains("/home/user/"), "{text}");
    assert!(text.contains("frontend-application"), "{text}");
    assert!(text.contains("Associate with a folder (n/a)"), "{text}");
    // The reason is bounded, marked where it was cut, and still readable.
    assert!(text.contains("Unavailable: explicit"), "{text}");
    let reason_lines = shown
        .iter()
        .skip_while(|l| !l.contains("Unavailable:"))
        .take_while(|l| !l.contains("Enter choose"))
        .filter(|l| !l.trim_matches(['│', ' ']).is_empty())
        .count();
    assert!((1..=REASON_LINES).contains(&reason_lines), "{text}");

    let mut dialog = cross();
    let text = lines(&mut dialog, 40, 20).join("\n");
    for row in [
        "Open original folder (n/a)",
        "Fork into current folder (n/a)",
    ] {
        assert!(text.contains(row), "{text}");
    }
}

/// A reason at the 512-character bound cannot push the footer off 80×24.
#[test]
fn the_longest_reason_is_bounded_at_every_size() {
    for (width, height) in [(80, 24), (40, 20), (30, 14)] {
        let mut long = decision(
            ResumeDecisionKind::HomeMissing,
            vec![
                offer(
                    ResumeAction::Locate,
                    false,
                    Some(&"reason words ".repeat(60)),
                ),
                offer(ResumeAction::ForkCurrent, false, None),
                offer(ResumeAction::Cancel, true, None),
            ],
        );
        let path = format!("/{}", "deep/".repeat(200));
        long.execution_path = Some(path.clone());
        let mut dialog = ResumeDecisionDialog::new(long);
        let shown = lines(&mut dialog, width, height);
        assert_whole(&shown);
        let text = shown.join("\n");
        assert!(text.contains('…'), "the cut is marked: {text}");
        // The folder keeps both its ends however few lines it is given (the
        // path itself is bounded to 512 characters first).
        let bounded: String = path.chars().take(512).collect();
        let folder: String = shown
            .iter()
            .map(|l| l.trim_matches(['│', ' ']))
            .filter(|l| l.contains("deep"))
            .collect();
        let (head, tail) = folder.split_once('…').expect("elided in the middle");
        assert!(bounded.starts_with(head) && head.len() > 3, "{folder}");
        assert!(bounded.ends_with(tail) && tail.len() > 3, "{folder}");
        for row in ["Locate folder", "Fork into current", "Cancel"] {
            assert!(text.contains(row), "{width}x{height}: {text}");
        }
    }
}

/// An unavailable offer with no reason still explains itself; on a tiny
/// terminal every unavailable row still carries a mark and nothing panics.
#[test]
fn tiny_terminals_keep_a_mark_on_every_unavailable_row() {
    for (width, height) in [(24, 12), (18, 10), (12, 8), (6, 5), (1, 1), (0, 0)] {
        let mut dialog = cross();
        dialog.handle_key(&Key::Down);
        let shown = lines(&mut dialog, width, height);
        assert!(!shown.is_empty());
        if width >= 18 && height >= 10 {
            let text = shown.join("\n");
            assert_eq!(text.matches('✗').count(), 2, "{width}x{height}: {text}");
            assert!(!text.contains("✗ Cancel"), "{text}");
        }
    }
    let mut dialog = cross();
    dialog.handle_key(&Key::Down);
    let text = lines(&mut dialog, 80, 24).join("\n");
    assert!(text.contains("Unavailable: not available yet"), "{text}");
    let mut unexplained = ResumeDecisionDialog::new(decision(
        ResumeDecisionKind::HomeChanged,
        vec![
            offer(ResumeAction::Locate, false, None),
            offer(ResumeAction::Cancel, true, None),
        ],
    ));
    let text = lines(&mut unexplained, 80, 24).join("\n");
    assert!(text.contains("Unavailable in this version"), "{text}");
}

/// Review R1-T6: bidi controls and zero-width characters never reach the
/// frame, at any width; nor can a newline forge a row.
#[test]
fn invisible_and_reordering_characters_never_reach_the_frame() {
    let invisible = [
        '\u{200b}', '\u{200c}', '\u{200d}', '\u{200e}', '\u{200f}', '\u{202a}', '\u{202b}',
        '\u{202c}', '\u{202d}', '\u{202e}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
        '\u{feff}', '\u{2060}',
    ];
    let soup: String = invisible.iter().flat_map(|ch| ['a', *ch]).collect();
    let mut hostile = decision(
        ResumeDecisionKind::CrossFolder,
        vec![
            offer(
                ResumeAction::OpenOriginal,
                false,
                Some(&format!("why{soup}\n→ Cancel")),
            ),
            offer(ResumeAction::Cancel, true, None),
        ],
    );
    hostile.session = format!("evil{soup}");
    hostile.execution_path = Some(format!("/srv/{soup}/gpj.exe"));
    hostile.detail = Some(soup.clone());
    let mut dialog = ResumeDecisionDialog::new(hostile);
    for (width, height) in [(80, 24), (40, 20), (20, 12)] {
        let (raw, _) = dialog.render_overlay(width, height);
        let raw = raw.join("\n");
        for ch in invisible {
            assert!(!raw.contains(ch), "U+{:04X} at {width}x{height}", ch as u32);
        }
        let text = crate::components::ansi::strip_ansi(&raw);
        // The newline in the reason forged no second cursor row.
        let cursors = text
            .lines()
            .filter(|l| l.trim_matches(['│', ' ']).starts_with('→'))
            .count();
        assert_eq!(cursors, 1, "{text}");
    }
    assert_eq!(
        crate::components::ansi::sanitize_untrusted_label(&format!("x{soup}"), 4),
        "xaaa"
    );
}
