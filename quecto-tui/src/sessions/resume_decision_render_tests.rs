//! The dialog on real terminal sizes (#2011 review R1-T2, R1-T3, R1-T6,
//! R2-T1, R2-T2): at 80×24 everything it exists to show is legible — every
//! reason the harness ships, in full; at 40×20 and below it degrades without
//! clipping its border. Sizes are TERMINAL sizes: the shell lays the dialog
//! over the whole frame.
use super::*;
use crate::components::utils::visible_width;

/// The harness's own reasons (`dto/resume_decision.rs::unavailable_reason`).
const OPEN_REASON: &str =
    "Not available yet. To continue this session, start quecto in that folder.";
const FORK_REASON: &str = "Not available yet. The saved conversation stays as it is.";
const LOCATE_REASON: &str = "Not available yet. The saved conversation is kept.";
const ASSOCIATE_REASON: &str = "Not available yet. This session predates folder tracking; \
    it stays listed under All Folders.";
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
    ResumeDecisionDialog::new(legacy, None)
}

fn cross() -> ResumeDecisionDialog {
    let mut cross = decision(
        ResumeDecisionKind::CrossFolder,
        vec![
            offer(ResumeAction::OpenOriginal, false, Some(OPEN_REASON)),
            offer(ResumeAction::ForkCurrent, false, Some(FORK_REASON)),
            offer(ResumeAction::Cancel, true, None),
        ],
    );
    cross.execution_path = Some(FOLDER.into());
    ResumeDecisionDialog::new(cross, None)
}

fn missing() -> ResumeDecisionDialog {
    let mut missing = decision(
        ResumeDecisionKind::HomeMissing,
        vec![
            offer(ResumeAction::Locate, false, Some(LOCATE_REASON)),
            offer(ResumeAction::ForkCurrent, false, Some(FORK_REASON)),
            offer(ResumeAction::Cancel, true, None),
        ],
    );
    missing.execution_path = Some(FOLDER.into());
    missing.detail = Some("Permission denied (os error 13)".into());
    ResumeDecisionDialog::new(missing, None)
}

/// The words between the box borders, re-joined: a wrapped text reads whole.
fn prose(lines: &[String]) -> String {
    let words = lines
        .iter()
        .flat_map(|l| l.trim_matches(['│', ' ']).split_whitespace());
    words.collect::<Vec<_>>().join(" ")
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
        text.contains("This session was saved before quecto tracked folders"),
        "{text}"
    );
    assert!(text.contains("chat-1750000000-ab"), "{text}");
    // The folder is on its own line(s), whole: both ends, no elision needed.
    let joined: String = shown.iter().map(|l| l.trim_matches(['│', ' '])).collect();
    assert!(joined.contains(FOLDER), "{text}");
    assert!(text.contains("Attach to a folder — unavailable"), "{text}");
    assert!(text.contains("your current session is untouched"), "{text}");

    let mut dialog = cross();
    let text = lines(&mut dialog, 80, 24).join("\n");
    for row in [
        "Open original folder — unavailable",
        "Copy into this folder as a new session — unavailable",
    ] {
        assert!(text.contains(row), "every row carries its mark: {text}");
    }
    assert!(text.contains("→ Open original folder"), "{text}");
}

/// Review R2-T1: every action is unavailable in this slice, so the reason IS
/// the dialog's content — each reason the harness ships is on screen in
/// full, un-elided, at the default 80×24 and at 60×24.
#[test]
fn every_shipped_reason_is_shown_in_full_at_80_by_24() {
    for (width, height) in [(80, 24), (60, 24)] {
        for (mut dialog, downs, reason) in [
            (cross(), 0, OPEN_REASON),
            (cross(), 1, FORK_REASON),
            (missing(), 0, LOCATE_REASON),
            (legacy(), 0, ASSOCIATE_REASON),
        ] {
            for _ in 0..downs {
                dialog.handle_key(&Key::Down);
            }
            let shown = lines(&mut dialog, width, height);
            assert_whole(&shown);
            let text = prose(&shown);
            assert!(text.contains(reason), "{width}x{height}: {shown:#?}");
            assert!(!text.contains('…'), "nothing is cut: {shown:#?}");
        }
    }
}

/// The reason takes the rows that are free — not a constant four: a long one
/// is whole where the terminal is tall, and is cut, marked, only by the rows.
#[test]
fn a_long_reason_uses_the_free_height_and_is_cut_only_by_it() {
    let reason = (1..=60).map(|n| format!("word{n}")).collect::<Vec<_>>();
    let reason = reason.join(" ");
    let long = || {
        ResumeDecisionDialog::new(
            decision(
                ResumeDecisionKind::LegacyUnscoped,
                vec![
                    offer(ResumeAction::Associate, false, Some(&reason)),
                    offer(ResumeAction::Cancel, true, None),
                ],
            ),
            None,
        )
    };
    let tall = lines(&mut long(), 80, 40);
    assert_whole(&tall);
    assert!(prose(&tall).contains(&reason), "{tall:#?}");
    let reason_rows = |shown: &[String]| shown.iter().filter(|l| l.contains("word")).count();
    assert!(
        reason_rows(&tall) > 4,
        "more than the old constant: {tall:#?}"
    );

    let short = lines(&mut long(), 40, 16);
    assert_whole(&short);
    let cut = short.iter().rfind(|l| l.contains("word")).unwrap();
    assert!(cut.trim_matches(['│', ' ']).ends_with('…'), "{short:#?}");
    assert!(prose(&short).contains("word1 word2"), "{short:#?}");
    assert!(!prose(&short).contains("word60"), "{short:#?}");
    // Title, folder and every offer are still there: the reason never starves them.
    let text = short.join("\n");
    for kept in ["/work/other", "Attach", "Cancel"] {
        assert!(text.contains(kept), "{kept}: {text}");
    }
}

/// Review R2-T2: a path longer than the 512-character bound keeps its TAIL —
/// two folders that differ only at the end never render alike.
#[test]
fn two_long_paths_that_differ_only_at_the_end_render_differently() {
    let shown = |last: &str| {
        let mut long = decision(
            ResumeDecisionKind::CrossFolder,
            vec![offer(ResumeAction::Cancel, true, None)],
        );
        let path = format!("/home/u/{}project-{last}", "deep/".repeat(120));
        assert_eq!(path.chars().count(), 619);
        long.execution_path = Some(path);
        let mut dialog = ResumeDecisionDialog::new(long, None);
        let stored = dialog.decision().execution_path.clone().unwrap();
        assert!(stored.chars().count() <= 512, "bounded: {}", stored.len());
        lines(&mut dialog, 80, 24).join("\n")
    };
    let (one, two) = (shown("ONE"), shown("TWO"));
    assert_ne!(one, two);
    assert!(
        one.contains("/home/u/deep/") && one.contains("project-ONE"),
        "{one}"
    );
    assert!(two.contains("project-TWO"), "{two}");
}

/// Review R2-T4: the harness's detail is shown under the folder, so a folder
/// that exists and cannot be read is not just called missing.
#[test]
fn the_detail_is_shown_under_the_folder() {
    let shown = lines(&mut missing(), 80, 24);
    let text = shown.join("\n");
    assert!(
        text.contains("can't be opened (missing, moved or no"),
        "{text}"
    );
    let folder = shown
        .iter()
        .rposition(|l| l.contains("frontend-application"));
    let detail = shown
        .iter()
        .position(|l| l.contains("Permission denied (os error 13)"));
    assert_eq!(detail, folder.map(|row| row + 1), "{text}");
    // One line, however long; and it goes before the folder does.
    let mut noisy = decision(
        ResumeDecisionKind::HomeMissing,
        vec![offer(ResumeAction::Cancel, true, None)],
    );
    noisy.detail = Some("denied ".repeat(60));
    let mut dialog = ResumeDecisionDialog::new(noisy, None);
    let shown = lines(&mut dialog, 80, 24);
    assert_eq!(shown.iter().filter(|l| l.contains("denied")).count(), 1);
    let small = lines(&mut dialog, 80, 10).join("\n");
    assert!(
        !small.contains("denied") && small.contains("/work/other"),
        "{small}"
    );
}

#[test]
fn words_are_never_broken_when_they_fit_the_line() {
    for (width, height) in [(80, 24), (60, 24), (40, 20), (30, 16)] {
        for mut dialog in [legacy(), cross(), missing()] {
            let shown = lines(&mut dialog, width, height);
            let known = [ASSOCIATE_REASON, OPEN_REASON, LOCATE_REASON].join(" ");
            let known: Vec<&str> = known.split_whitespace().collect();
            // The reason: what lies between the last offer and the footer.
            let after_offers = shown.iter().skip_while(|l| !l.contains("Cancel")).skip(1);
            for line in after_offers.take_while(|l| !l.contains("Esc cancel")) {
                for word in line.trim_matches(['│', ' ']).split_whitespace() {
                    let word = word.trim_end_matches('…');
                    let whole = word.is_empty()
                        || known.contains(&word)
                        || !known.iter().any(|w| w.starts_with(word));
                    assert!(whole, "{width}x{height}: {word:?} in {line:?}");
                }
            }
        }
    }
}

/// Review R2-T3: a 40-column terminal is legible, not merely unclipped — a
/// title that says what happened, whole labels (or a marked cut), the mark on
/// every unavailable row, the reason in full.
#[test]
fn at_40_by_20_it_is_legible_and_keeps_its_border() {
    let mut dialog = legacy();
    let shown = lines(&mut dialog, 40, 20);
    assert_whole(&shown);
    let text = shown.join("\n");
    assert!(
        prose(&shown).contains("This session was saved before quecto tracked folders"),
        "{text}"
    );
    // The folder keeps BOTH ends on at most two lines, elided in the middle.
    assert!(text.contains("/home/user/"), "{text}");
    assert!(text.contains("frontend-application"), "{text}");
    assert!(text.contains("Attach to a folder (n/a)"), "{text}");
    assert!(prose(&shown).contains(ASSOCIATE_REASON), "{text}");

    let mut dialog = cross();
    let shown = lines(&mut dialog, 40, 20);
    assert_whole(&shown);
    let text = shown.join("\n");
    for row in ["Open original — unavailable", "Copy here — unavailable"] {
        assert!(text.contains(row), "{text}");
    }
    assert!(
        prose(&shown).contains("This session belongs to another folder"),
        "{text}"
    );
    assert!(prose(&shown).contains(OPEN_REASON), "{text}");
}

/// Narrower still, the title takes its short form instead of "This session…".
#[test]
fn a_narrow_terminal_gets_the_short_title_and_marked_cuts() {
    let shown = lines(&mut missing(), 26, 16);
    assert_whole(&shown);
    let text = shown.join("\n");
    assert!(text.contains("Folder missing"), "{text}");
    assert!(!text.contains("This session…"), "{text}");
    assert!(
        text.contains("→ Locate (n/a)") && text.contains("Copy here (n/a)"),
        "{text}"
    );
    // Narrower than the short labels: every cut is marked.
    let shown = lines(&mut cross(), 16, 16);
    let text = shown.join("\n");
    assert!(
        text.contains("→ ✗ Ope…") && text.contains("✗ Cop…"),
        "{text}"
    );
}

/// A reason at the 512-character bound cannot push the border off any size.
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
        let mut dialog = ResumeDecisionDialog::new(long, None);
        let shown = lines(&mut dialog, width, height);
        assert_whole(&shown);
        let text = shown.join("\n");
        assert!(text.contains('…'), "the cut is marked: {text}");
        // The folder keeps both its ends however few lines it is given.
        let folder: String = shown
            .iter()
            .map(|l| l.trim_matches(['│', ' ']))
            .filter(|l| l.contains("deep"))
            .collect();
        let (head, tail) = folder.split_once('…').expect("elided in the middle");
        assert!(path.starts_with(head) && head.len() > 3, "{folder}");
        assert!(path.ends_with(tail) && tail.len() > 3, "{folder}");
        for row in ["Locate", "Copy", "Cancel"] {
            assert!(text.contains(row), "{width}x{height}: {text}");
        }
    }
}

/// Review R2-T12: more offers than today's three, or a very low terminal,
/// never clip the bottom border — the list scrolls in fewer rows and the
/// cursor's row stays on screen.
#[test]
fn many_offers_or_a_low_terminal_keep_the_box_whole() {
    let many = || {
        let all = [
            ResumeAction::OpenOriginal,
            ResumeAction::ForkCurrent,
            ResumeAction::Locate,
            ResumeAction::Associate,
        ];
        let mut offers: Vec<_> = (all.iter().chain(&all))
            .map(|action| offer(*action, false, Some(OPEN_REASON)))
            .collect();
        offers.push(offer(ResumeAction::Cancel, true, None));
        ResumeDecisionDialog::new(decision(ResumeDecisionKind::CrossFolder, offers), None)
    };
    for height in 7..=24 {
        for (mut dialog, downs) in [(many(), 0), (many(), 8), (cross(), 2), (legacy(), 0)] {
            for _ in 0..downs {
                dialog.handle_key(&Key::Down);
            }
            let shown = lines(&mut dialog, 80, height);
            assert!(shown[0].starts_with('┌'), "{height}: {shown:#?}");
            let last = shown.last().unwrap();
            assert!(last.starts_with('└'), "{height}: {shown:#?}");
            assert_eq!(
                shown.iter().filter(|l| l.contains('→')).count(),
                1,
                "{height}: {shown:#?}"
            );
        }
    }
    // Growing back restores every row.
    let mut dialog = many();
    lines(&mut dialog, 80, 8);
    let text = lines(&mut dialog, 80, 40).join("\n");
    assert_eq!(text.matches("unavailable").count(), 8, "{text}");
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
    assert!(text.contains(FORK_REASON), "{text}");
    let mut unexplained = ResumeDecisionDialog::new(
        decision(
            ResumeDecisionKind::HomeChanged,
            vec![
                offer(ResumeAction::Locate, false, None),
                offer(ResumeAction::Cancel, true, None),
            ],
        ),
        None,
    );
    let text = lines(&mut unexplained, 80, 24).join("\n");
    assert!(text.contains("Not available in this version"), "{text}");
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
    let mut dialog = ResumeDecisionDialog::new(hostile, Some(&format!("title{soup}")));
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
