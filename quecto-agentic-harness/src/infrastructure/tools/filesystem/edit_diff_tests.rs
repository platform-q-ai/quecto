use super::*;

/// The diff of two texts, with the change between their common prefix and
/// suffix, as an edit's narrowed splice gives it.
fn make_edit_diff(path: &str, old: &str, new: &str) -> String {
    super::make_edit_diff(path, old, new, &Change::between(old, new))
}

#[test]
fn test_over_cap_edit_diff_returns_bounded_concrete_context_and_notice() {
    let old = (0..900)
        .map(|i| format!("old line {i:03}"))
        .collect::<Vec<_>>()
        .join("\n");
    let new = (0..900)
        .map(|i| format!("new line {i:03}"))
        .collect::<Vec<_>>()
        .join("\n");

    let diff = make_edit_diff("large.txt", &old, &new);

    assert!(
        diff.len() <= DIFF_MAX_BYTES,
        "diff exceeded cap: {}",
        diff.len()
    );
    assert!(diff.starts_with("Successfully edited large.txt\n\n-  1 old line 000"));
    assert!(diff.contains("-  2 old line 001"));
    assert!(diff.contains("[diff truncated: 0 of 1 hunks shown, 1800 lines changed total]"));
    assert_ne!(diff, "Successfully edited large.txt");
}

#[test]
fn test_over_cap_multihunk_notice_counts_completed_hunks() {
    let old_lines = (0..240).map(|i| format!("line {i:03}")).collect::<Vec<_>>();
    let mut new_lines = old_lines.clone();
    new_lines[10] = "changed first hunk".to_string();
    new_lines[80] = "changed second hunk".to_string();
    for (i, line) in new_lines.iter_mut().enumerate().skip(160).take(60) {
        *line = format!("changed large hunk {i:03} {}", "z".repeat(60));
    }

    let diff = make_edit_diff("multi.txt", &old_lines.join("\n"), &new_lines.join("\n"));

    assert!(
        diff.len() <= DIFF_MAX_BYTES,
        "diff exceeded cap: {}",
        diff.len()
    );
    assert!(diff.contains("- 11 line 010"));
    assert!(diff.contains("+ 11 changed first hunk"));
    assert!(diff.contains("[diff truncated: 2 of 3 hunks shown, 124 lines changed total]"));
}

/// #2194: a line changed whole is shown by its two ends, so the diff is
/// not truncated.
#[test]
fn test_single_over_cap_line_still_shows_concrete_diff_prefix() {
    let old = format!("old {}\n", "x".repeat(DIFF_MAX_BYTES));
    let new = format!("new {}\n", "y".repeat(DIFF_MAX_BYTES));

    let diff = make_edit_diff("long-line.txt", &old, &new);

    assert!(diff.len() <= 1024, "diff not windowed: {}", diff.len());
    assert!(diff.starts_with("Successfully edited long-line.txt\n\n-1 old xxx"));
    assert!(diff.contains("\n+1 new y"));
    // Nothing is shared, so the lines are wholly changed: both ends, no note.
    assert!(diff.contains("xxx\u{2026}xxx"), "{diff}");
    assert!(!diff.contains("change starts"), "{diff}");
    assert!(!diff.contains("[diff truncated"), "{diff}");
}

#[test]
fn test_multibyte_over_cap_diff_truncates_on_char_boundary() {
    let multibyte = "界".repeat(DIFF_MAX_BYTES);
    let old = format!("old {multibyte}\n");
    let new = format!("new {multibyte}\n");
    let path = format!("emoji-{}", "🚀".repeat(DIFF_MAX_BYTES));

    let diff = make_edit_diff(&path, &old, &new);

    assert!(
        diff.len() <= DIFF_MAX_BYTES,
        "diff exceeded cap: {}",
        diff.len()
    );
    assert!(diff.is_char_boundary(diff.len()));
    assert!(diff.contains("\n\n-1 old 界"));
    assert!(diff.contains("\n+1 new"));
    assert!(diff.contains("[diff truncated: "), "{diff}");
}

#[test]
fn test_long_path_over_cap_diff_stays_bounded_with_notice() {
    let long_path = "p".repeat(DIFF_MAX_BYTES);
    let old = format!("old {}\n", "x".repeat(DIFF_MAX_BYTES));
    let new = format!("new {}\n", "y".repeat(DIFF_MAX_BYTES));

    let diff = make_edit_diff(&long_path, &old, &new);

    assert!(
        diff.len() <= DIFF_MAX_BYTES,
        "diff exceeded cap: {}",
        diff.len()
    );
    assert!(diff.starts_with("Successfully edited ppp"));
    assert!(diff.contains("\n\n-1 old x"));
    assert!(diff.contains("[diff truncated: "), "{diff}");
}

#[test]
fn test_small_edit_diff_behavior_is_unchanged_without_truncation_notice() {
    let diff = make_edit_diff("small.txt", "line1\nline2\n", "line1\nCHANGED\n");

    assert_eq!(
        diff,
        "Successfully edited small.txt\n\n 1 line1\n-2 line2\n+2 CHANGED"
    );
}

// --- #2194: a long line is shown as a window around its change ---

/// The text a rendered line shows, without its marker, number and "…".
fn shown_parts(diff: &str, marker: char) -> Vec<String> {
    diff.lines()
        .filter(|line| line.starts_with(marker))
        .map(|line| line.split_once(' ').map_or("", |(_, text)| text))
        .flat_map(|text| text.split('\u{2026}'))
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

/// #2194 (B19): a change at the end of a 1 MiB line is shown.
#[test]
fn a_change_at_the_end_of_a_huge_line_is_shown() {
    let old = format!("{}ENDING!", "a".repeat(1_048_569));
    let new = format!("{}FINISH!", "a".repeat(1_048_569));

    let diff = make_edit_diff("large.txt", &old, &new);

    assert!(diff.len() <= 600, "{} bytes: {diff}", diff.len());
    assert!(diff.contains("\n-1 \u{2026}aaa"), "{diff}");
    assert!(diff.contains("aaaENDING!\n"), "{diff}");
    assert!(diff.contains("\n+1 \u{2026}aaa"), "{diff}");
    assert!(diff.contains("aaaFINISH!\n"), "{diff}");
    assert!(diff.contains("change starts at column 1048570"), "{diff}");
    assert!(diff.contains("line 1 is 1048576 bytes"), "{diff}");
    assert!(
        !diff.contains("was"),
        "a length that did not change: {diff}"
    );
    assert!(!diff.contains("[diff truncated"), "{diff}");
}

#[test]
fn a_change_in_the_middle_of_a_long_line_cuts_both_sides() {
    let old = format!("{}OLD{}", "p".repeat(3000), "s".repeat(3000));
    let new = format!("{}NEWER{}", "p".repeat(3000), "s".repeat(3000));

    let diff = make_edit_diff("mid.txt", &old, &new);

    assert!(diff.len() <= 600, "{} bytes: {diff}", diff.len());
    assert!(diff.contains("-1 \u{2026}ppp"), "{diff}");
    assert!(diff.contains("pppOLDsss"), "{diff}");
    assert!(diff.contains("+1 \u{2026}ppp"), "{diff}");
    assert!(diff.contains("pppNEWERsss"), "{diff}");
    let removed = diff.lines().find(|l| l.starts_with('-')).unwrap();
    assert!(removed.ends_with('\u{2026}'), "{removed}");
    assert!(diff.contains("change starts at column 3001"), "{diff}");
    assert!(diff.contains("was 6003 bytes"), "{diff}");
}

/// A long changed span keeps both its ends and drops its middle.
#[test]
fn a_long_changed_span_shows_both_of_its_ends() {
    let old = format!("keep {}{} keep", "B".repeat(2000), "E".repeat(2000));
    let new = format!("keep {}{} keep", "b".repeat(2000), "e".repeat(2000));

    let diff = make_edit_diff("span.txt", &old, &new);

    assert!(diff.len() <= 800, "{} bytes: {diff}", diff.len());
    assert!(diff.contains("-1 keep BBB"), "{diff}");
    assert!(diff.contains("EEE keep\n"), "{diff}");
    assert!(diff.contains("+1 keep bbb"), "{diff}");
    assert!(diff.contains("eee keep\n"), "{diff}");
}

/// Every cut is on a character boundary, and every shown piece is text of
/// the line it came from (#2223: multibyte text is never split).
#[test]
fn a_window_on_multibyte_text_cuts_on_character_boundaries() {
    for (unit, before, after) in [
        ("\u{754C}", "X", "Y"),
        ("\u{1F680}", "a", "bc"),
        ("\u{E9}", "\u{1F680}", "\u{754B}"),
    ] {
        for pad in [0usize, 1, 2, 3] {
            // The pad sits next to the change, so the window's edges, a fixed
            // number of bytes away, land inside a character.
            let (pad, units) = ("z".repeat(pad), unit.repeat(900));
            let old = format!("{units}{pad}{before}{pad}{units}");
            let new = format!("{units}{pad}{after}{pad}{units}");
            let diff = make_edit_diff("mb.txt", &old, &new);
            assert!(diff.len() <= 800, "{} bytes", diff.len());
            for part in shown_parts(&diff, '-') {
                assert!(old.contains(&part), "{part:?} is not in the old line");
            }
            for part in shown_parts(&diff, '+') {
                assert!(new.contains(&part), "{part:?} is not in the new line");
            }
            assert!(diff.contains(before) && diff.contains(after), "{diff}");
            // Context is shown on both sides of the change, not dropped.
            let lead = format!("{}{pad}", unit.repeat(10));
            let trail = format!("{pad}{}", unit.repeat(5));
            assert!(diff.contains(&format!("{lead}{before}{trail}")), "{diff}");
            assert!(diff.contains(&format!("{lead}{after}{trail}")), "{diff}");
        }
    }
}

/// A short line is shown whole, even beside a long one.
#[test]
fn short_lines_of_a_multi_line_edit_are_shown_whole() {
    let long_old = format!("{}x", "q".repeat(500));
    let long_new = format!("{}y", "q".repeat(500));
    let old = format!("head\n{long_old}\nshort one\ntail\n");
    let new = format!("head\n{long_new}\nshort two\ntail\n");

    let diff = make_edit_diff("multi.txt", &old, &new);

    assert!(diff.contains("\n 1 head\n"), "{diff}");
    assert!(diff.contains("\n-2 \u{2026}qqq"), "{diff}");
    assert!(diff.contains("qqqx\n"), "{diff}");
    assert!(diff.contains("\n-3 short one\n"), "{diff}");
    assert!(diff.contains("\n+2 \u{2026}qqq"), "{diff}");
    assert!(diff.contains("qqqy\n"), "{diff}");
    assert!(diff.contains("\n+3 short two\n"), "{diff}");
    assert!(diff.contains("\n 4 tail"), "{diff}");
    assert!(diff.len() <= 800, "{} bytes: {diff}", diff.len());
}

/// A long context line shows its start; a long added line is all change,
/// so it shows both of its ends. Neither can fill the diff.
#[test]
fn long_context_lines_show_their_start_and_added_ones_their_ends() {
    let context = format!("ctx{}", "c".repeat(5000));
    let added = format!("add{}", "n".repeat(5000));
    let old = format!("{context}\nmid\n");
    let new = format!("{context}\nmid\n{added}\n");

    let diff = make_edit_diff("ctx.txt", &old, &new);

    assert!(diff.len() <= 800, "{} bytes: {diff}", diff.len());
    assert!(diff.contains("\n 1 ctxccc"), "{diff}");
    assert!(diff.contains("\n+3 addnnn"), "{diff}");
    let added_line = diff.lines().find(|l| l.starts_with("+3")).unwrap();
    assert!(added_line.contains('\u{2026}'), "{added_line}");
    assert!(!diff.contains("[diff truncated"), "{diff}");
}

/// Text added to the end of a long line: an empty old span is fine.
#[test]
fn text_appended_to_a_long_line_is_shown() {
    let old = "w".repeat(1000);
    let new = format!("{}TAIL", "w".repeat(1000));

    let diff = make_edit_diff("append.txt", &old, &new);

    assert!(diff.contains("-1 \u{2026}www"), "{diff}");
    assert!(diff.contains("+1 \u{2026}www"), "{diff}");
    assert!(diff.contains("wwwTAIL\n"), "{diff}");
    assert!(diff.contains("change starts at column 1001"), "{diff}");
}

#[test]
fn common_prefix_and_suffix_never_overlap() {
    assert_eq!(changed_spans("aXa", "aXXa"), (2..2, 2..3));
    assert_eq!(changed_spans("aaaa", "aaaaa"), (4..4, 4..5));
    assert_eq!(changed_spans("same", "same"), (4..4, 4..4));
    assert_eq!(changed_spans("\u{754C}a", "\u{754B}a"), (0..3, 0..3));
    assert_eq!(changed_spans("a\u{754C}", "a\u{754B}"), (1..4, 1..4));
}

/// One long line is enough: a short line grown long is windowed too.
#[test]
fn a_short_line_grown_long_is_windowed() {
    let old = "abc";
    let new = format!("abc{}", "x".repeat(500));

    let diff = make_edit_diff("grow.txt", old, &new);

    assert!(diff.contains("\n-1 abc\n"), "{diff}");
    assert!(diff.contains("change starts at column 4"), "{diff}");
    assert!(diff.contains("line 1 is 503 bytes, was 3 bytes"), "{diff}");
    assert!(diff.len() <= 600, "{} bytes: {diff}", diff.len());
}

// --- #2194 review: windows follow the edit's change, not line pairs ---

/// The diff an edit makes: `old` with `splice` replaced by `inserted`.
fn edit_diff(old: &str, splice: Range<usize>, inserted: &str) -> String {
    let new = format!("{}{inserted}{}", &old[..splice.start], &old[splice.end..]);
    let change = Change::of_splice(old, &new, splice, inserted.len());
    super::make_edit_diff("f.txt", old, &new, &change)
}

/// The review's case: a header line added before a long line whose end
/// changes. The long line is not paired with the header.
#[test]
fn a_line_added_before_a_changed_long_line_does_not_hide_the_change() {
    let long = format!("{}ENDING!", "a".repeat(2000));
    let diff = edit_diff(
        &long,
        0..long.len(),
        &format!("// header\n{}FINISH!", "a".repeat(2000)),
    );

    assert!(diff.contains("\n+1 // header\n"), "{diff}");
    assert!(diff.contains("aaaENDING!"), "{diff}");
    assert!(diff.contains("aaaFINISH!"), "{diff}");
    assert!(diff.len() <= 800, "{} bytes: {diff}", diff.len());
}

#[test]
fn a_line_added_after_a_changed_long_line_does_not_hide_the_change() {
    let old = format!("{}OLD{}\nnext\n", "p".repeat(900), "s".repeat(900));
    let at = old.find("OLD").unwrap();
    let diff = edit_diff(
        &old,
        at..at + 3,
        &format!("NEWER{}\nadded", "s".repeat(900)),
    );

    assert!(diff.contains("pppOLDsss"), "{diff}");
    assert!(diff.contains("pppNEWERsss"), "{diff}");
    assert!(diff.contains("\n+2 added"), "{diff}");
    assert!(diff.contains("the change starts at column 901"), "{diff}");
    // The long line is paired with its own new text, not the next line.
    assert!(
        diff.contains("line 1 is 1805 bytes, was 1803 bytes"),
        "{diff}"
    );
}

#[test]
fn a_short_line_removed_next_to_a_changed_long_line_does_not_hide_the_change() {
    let old = format!("gone\n{}OLD{}\n", "p".repeat(900), "s".repeat(900));
    let at = old.find("OLD").unwrap();
    let diff = edit_diff(&old, 0..at + 3, &format!("{}NEW", "p".repeat(900)));

    assert!(diff.contains("\n-1 gone\n"), "{diff}");
    assert!(diff.contains("pppOLDsss"), "{diff}");
    assert!(diff.contains("pppNEWsss"), "{diff}");
}

/// A long line that the change does not touch shows its start, and no note
/// is made for it.
#[test]
fn a_long_line_outside_the_change_gets_no_window_or_note() {
    let long = format!("{}tail", "k".repeat(900));
    let old = format!("{long}\nx\n");
    let diff = edit_diff(&old, old.len() - 2..old.len() - 1, "y\nz");

    assert!(
        diff.contains(&format!("\n 1 {}\u{2026}", "k".repeat(120))),
        "{diff}"
    );
    assert!(!diff.contains("the change starts"), "{diff}");
}

/// The byte cap keeps a note with its line: they are kept or cut together.
#[test]
fn the_byte_cap_never_separates_a_note_from_its_line() {
    for count in 20..70 {
        let old: String = (0..count)
            .map(|i| format!("{}{i:03}END\n", "v".repeat(300)))
            .collect();
        let new: String = (0..count)
            .map(|i| format!("{}{i:03}FIN\n", "v".repeat(300)))
            .collect();
        let diff = make_edit_diff("cap.txt", &old, &new);
        assert!(
            diff.len() <= DIFF_MAX_BYTES,
            "{count}: {} bytes",
            diff.len()
        );
        let has_line = diff.contains("\n+ 1 ") || diff.contains("\n+1 ");
        assert_eq!(
            has_line,
            diff.contains("[the change starts"),
            "{count}: {diff}"
        );
    }
}

#[test]
fn a_splice_is_narrowed_to_what_differs() {
    let change = Change::of_splice("xxabcyy", "xxaZcyy", 2..5, 3);
    assert_eq!(
        change,
        Change {
            old: 3..4,
            new: 3..4
        }
    );
    let change = Change::of_splice("ab", "aXb", 1..1, 1);
    assert_eq!(
        change,
        Change {
            old: 1..1,
            new: 1..2
        }
    );
}

/// Empty lines before the change still place it: line starts count every
/// line's newline.
#[test]
fn empty_lines_before_a_long_line_keep_the_column_right() {
    let old = format!("\n\n{}OLD{}\n", "p".repeat(900), "s".repeat(900));
    let at = old.find("OLD").unwrap();
    let diff = edit_diff(&old, at..at + 3, "NEW");

    assert!(diff.contains("\n-3 \u{2026}ppp"), "{diff}");
    assert!(diff.contains("pppOLDsss"), "{diff}");
    assert!(diff.contains("the change starts at column 901"), "{diff}");
}

/// A changed long line that the change only borders shows its start: the
/// line diff marks it changed ("L\n" became "L"), but no text of it changed.
#[test]
fn a_long_line_the_change_only_borders_shows_its_start() {
    let long = format!("S{}E", "m".repeat(300));
    let old = format!("{long}\n{long}");
    let diff = edit_diff(&old, long.len()..old.len(), "");

    assert!(diff.contains("\n-1 Smmm"), "{diff}");
    assert!(!diff.contains("the change starts"), "{diff}");
}

// --- #2194 review 2: each changed line is windowed on its own difference ---

fn long_line(tag: &str) -> String {
    format!("{}{tag}{}", "p".repeat(400), "s".repeat(400))
}

/// The review's case: three long lines each changed in the middle. Every
/// line shows its own change, and each note names its own line.
#[test]
fn each_of_three_changed_long_lines_shows_its_own_change() {
    let old = [long_line("K1"), long_line("K2"), long_line("K3")].join("\n");
    let new = [long_line("N1x"), long_line("N2"), long_line("N3yy")].join("\n");
    let diff = edit_diff(&old, 0..old.len(), &new);

    for (n, was, now) in [
        (1, "pK1s", "pN1xs"),
        (2, "pK2s", "pN2s"),
        (3, "pK3s", "pN3yys"),
    ] {
        let removed = diff
            .lines()
            .find(|l| l.starts_with(&format!("-{n} ")))
            .unwrap();
        let added = diff
            .lines()
            .find(|l| l.starts_with(&format!("+{n} ")))
            .unwrap();
        assert!(removed.contains(was), "{n}: {removed}");
        assert!(added.contains(now), "{n}: {added}");
        assert!(
            diff.contains(&format!("column 401; line {n} is")),
            "{n}: {diff}"
        );
    }
}

/// Sixty changed long lines: every shown line carries its own change, and
/// the diff stays in its cap.
#[test]
fn sixty_changed_long_lines_each_show_their_own_change() {
    let old: Vec<String> = (0..60).map(|i| long_line(&format!("{i:03}END"))).collect();
    let new: Vec<String> = (0..60).map(|i| long_line(&format!("{i:03}FIN"))).collect();
    let (old, new) = (old.join("\n"), new.join("\n"));
    let diff = edit_diff(&old, 0..old.len(), &new);

    assert!(diff.len() <= DIFF_MAX_BYTES, "{} bytes", diff.len());
    let mut shown = 0;
    for line in diff.lines() {
        let Some((head, _)) = line.split_once(' ') else {
            continue;
        };
        let Ok(n) = head[1..].parse::<usize>() else {
            continue;
        };
        match head.chars().next() {
            Some('-') => assert!(line.contains(&format!("p{:03}ENDs", n - 1)), "{line}"),
            Some('+') => assert!(line.contains(&format!("p{:03}FINs", n - 1)), "{line}"),
            _ => continue,
        }
        shown += 1;
    }
    assert!(shown >= 20, "only {shown} lines shown: {diff}");
}

/// #2194 review 2: a long line added whole shows its two ends, no note.
#[test]
fn a_long_line_added_whole_shows_both_ends_and_no_note() {
    let old = "a\nb\n";
    let added = format!("S{}E", "n".repeat(900));
    let diff = edit_diff(old, 2..2, &format!("{added}\n"));

    assert!(
        diff.contains(&format!("\n+2 S{}\u{2026}", "n".repeat(59))),
        "{diff}"
    );
    assert!(diff.contains(&format!("{}E\n", "n".repeat(59))), "{diff}");
    assert!(!diff.contains("change starts"), "{diff}");
}

/// #2194 review 2: a newline added at the end of a long last line shows the
/// same end on both sides and says only the line break changed.
#[test]
fn a_newline_added_after_a_long_last_line_is_named() {
    let old = format!("{}END", "q".repeat(600));
    let diff = edit_diff(&old, old.len()..old.len(), "\n");

    let removed = diff.lines().find(|l| l.starts_with("-1 ")).unwrap();
    let added = diff.lines().find(|l| l.starts_with("+1 ")).unwrap();
    assert_eq!(removed[1..], added[1..], "{diff}");
    assert!(removed.ends_with("qqqEND"), "{diff}");
    assert!(
        diff.contains("[the text of line 1 is unchanged: only its line break changed]"),
        "{diff}"
    );
}

#[test]
fn partners_are_found_in_order_by_what_they_share() {
    let (a, b) = (long_line("A"), long_line("B"));
    let other = "z".repeat(500);
    let old = format!("{a}\n{b}\n");
    let new = format!("{other}\n{a}X\n{b}Y\n");
    let diff = TextDiff::from_lines(old.as_str(), new.as_str());
    let old_side = Side::of(diff.old_slices(), &old, 0..0);
    let new_side = Side::of(diff.new_slices(), &new, 0..0);
    assert_eq!(
        partners(&old_side, &new_side, 0..2, 0..3),
        vec![(0, 1), (1, 2)]
    );
}

/// The partner is the line that shares the most, not the first that
/// shares anything.
#[test]
fn the_partner_that_shares_most_wins_over_an_earlier_one() {
    let a = long_line("A");
    let weak = format!("p{}", "z".repeat(500));
    let old = format!("{a}\n");
    let new = format!("{weak}\n{a}X\n");
    let diff = TextDiff::from_lines(old.as_str(), new.as_str());
    let old_side = Side::of(diff.old_slices(), &old, 0..0);
    let new_side = Side::of(diff.new_slices(), &new, 0..0);
    assert_eq!(partners(&old_side, &new_side, 0..1, 0..2), vec![(0, 1)]);
}

/// Short lines are never paired or windowed: they are shown whole.
#[test]
fn a_short_changed_pair_is_shown_whole_without_a_note() {
    let diff = make_edit_diff("s.txt", "hello world\n", "hello there\n");
    assert_eq!(
        diff,
        "Successfully edited s.txt\n\n-1 hello world\n+1 hello there"
    );
}

/// An unpaired line that holds only part of the change is windowed on that
/// part, with a note: here a long line split in two by inserted text.
#[test]
fn an_unpaired_line_partly_changed_is_windowed_on_its_part() {
    let old = format!("{}{}", "P".repeat(300), "S".repeat(300));
    let inserted = format!("{}\n{}", "X".repeat(300), "Y".repeat(300));
    let diff = edit_diff(&old, 300..300, &inserted);

    let added = diff.lines().find(|l| l.starts_with("+2 ")).unwrap();
    assert!(added.starts_with("+2 YYY"), "{diff}");
    assert!(added.contains(&format!("Y{}", "S".repeat(60))), "{diff}");
    assert!(diff.contains("column 1; line 2 is 600 bytes"), "{diff}");
}
