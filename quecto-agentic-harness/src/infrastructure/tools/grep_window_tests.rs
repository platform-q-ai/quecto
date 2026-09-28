use super::*;
use crate::infrastructure::tools::grep::grep_text::Decoded;

const BUDGET: usize = 500;

/// `show_line` on a line of valid UTF-8.
fn show(line: &str, hits: &[Range<usize>], budget: usize) -> (String, bool) {
    show_line(&Decoded::new(line.as_bytes()), hits, budget)
}

/// One match's range, as a slice of hits.
fn one(hit: Range<usize>) -> Vec<Range<usize>> {
    vec![hit]
}

/// The text a shown line keeps of `line`, and the bytes its markers say
/// were left out: together they account for the whole line.
fn accounted(shown: &str) -> (usize, usize) {
    let body = shown
        .rsplit_once(" [line is ")
        .map_or(shown, |(body, _)| body);
    let mut kept = 0;
    let mut left_out = 0;
    let mut rest = body;
    while let Some(at) = rest.find("…[") {
        kept += at;
        let after = &rest[at + "…[".len()..];
        let (number, tail) = after.split_once(" bytes]…").expect("a marker is closed");
        left_out += number.parse::<usize>().expect("a marker counts bytes");
        rest = tail;
    }
    (kept + rest.len(), left_out)
}

fn assert_accounts_for(shown: &str, line: &str) {
    let (kept, left_out) = accounted(shown);
    assert_eq!(
        kept + left_out,
        line.len(),
        "every byte is shown or counted: {shown}"
    );
    assert!(
        kept <= BUDGET,
        "at most {BUDGET} bytes of the line are shown, got {kept}"
    );
}

#[test]
fn a_line_within_the_budget_is_shown_whole() {
    let (shown, cut) = show("fn needle() {}", &one(3..9), BUDGET);
    assert_eq!(shown, "fn needle() {}");
    assert!(!cut);
}

/// #2201: the match past byte 500 of a 2 MB line is what is shown, with
/// where it is.
#[test]
fn a_match_in_the_middle_of_a_long_line_is_shown_with_its_offset() {
    let line = format!("{}needle{}", "a".repeat(1 << 20), "b".repeat(1 << 20));
    let at = 1 << 20;
    let (shown, cut) = show(&line, &one(at..at + 6), BUDGET);
    assert!(cut);
    assert!(
        shown.contains("aneedleb"),
        "the match and its context: {shown}"
    );
    assert!(
        shown.starts_with("…["),
        "text before the window is marked: {shown}"
    );
    assert!(
        shown.ends_with(" [line is 2.0MB; match at byte 1048576]"),
        "{shown}"
    );
    assert_accounts_for(&shown, &line);
    let (kept, _) = accounted(&shown);
    assert_eq!(kept, BUDGET, "a lone match gets the whole budget");
}

#[test]
fn a_match_at_the_start_of_a_long_line_keeps_the_line_start() {
    let line = format!("needle{}", "a".repeat(5000));
    let (shown, _) = show(&line, &one(0..6), BUDGET);
    assert!(shown.starts_with("needleaaa"), "{shown}");
    assert!(shown.contains("aaa…[4506 bytes]…"), "{shown}");
    assert!(shown.ends_with("; match at byte 0]"), "{shown}");
    assert_accounts_for(&shown, &line);
}

#[test]
fn a_match_at_the_end_of_a_long_line_keeps_the_line_end() {
    let line = format!("{}needle", "a".repeat(5000));
    let (shown, _) = show(&line, &one(5000..5006), BUDGET);
    assert!(shown.starts_with("…[4506 bytes]…aaa"), "{shown}");
    assert!(shown.contains("aaaneedle [line is"), "{shown}");
    assert_accounts_for(&shown, &line);
}

#[test]
fn close_matches_share_one_window() {
    let line = format!("{}one two{}", "a".repeat(3000), "a".repeat(3000));
    let (shown, _) = show(&line, &[3000..3003, 3004..3007], BUDGET);
    assert_eq!(shown.matches("…[").count(), 2, "one window: {shown}");
    assert!(shown.contains("one two"), "{shown}");
    assert!(
        shown.ends_with("; 2 matches, first at byte 3000]"),
        "{shown}"
    );
    assert_accounts_for(&shown, &line);
}

#[test]
fn distant_matches_each_get_a_window() {
    let line = format!(
        "{}first{}second{}",
        "a".repeat(3000),
        "b".repeat(3000),
        "c".repeat(3000)
    );
    let second = 3000 + 5 + 3000;
    let (shown, _) = show(&line, &[3000..3005, second..second + 6], BUDGET);
    assert!(shown.contains("afirstb"), "{shown}");
    assert!(shown.contains("bsecondc"), "{shown}");
    assert_eq!(shown.matches("…[").count(), 3, "two windows: {shown}");
    assert!(
        shown.ends_with("; 2 matches, first at byte 3000]"),
        "{shown}"
    );
    assert_accounts_for(&shown, &line);
}

#[test]
fn matches_past_the_window_limit_are_counted_as_not_shown() {
    let spacing = 2000;
    let line = "x".repeat(spacing * 10);
    let hits: Vec<_> = (0..10)
        .map(|i| i * spacing + 1000..i * spacing + 1001)
        .collect();
    assert_eq!(
        MAX_WINDOWS, 4,
        "#2201: a long line shows up to four windows"
    );
    let (shown, _) = show(&line, &hits, BUDGET);
    assert_eq!(
        shown.matches("…[").count(),
        MAX_WINDOWS + 1,
        "at most {MAX_WINDOWS} windows: {shown}"
    );
    assert!(
        shown.ends_with(&format!(
            "; 10 matches, first at byte 1000; {} not shown]",
            10 - MAX_WINDOWS
        )),
        "{shown}"
    );
    assert_accounts_for(&shown, &line);
}

#[test]
fn a_match_longer_than_the_budget_shows_its_start() {
    let line = format!(
        "{}{}{}",
        "a".repeat(1000),
        "M".repeat(4000),
        "a".repeat(1000)
    );
    let (shown, _) = show(&line, &one(1000..5000), BUDGET);
    assert!(
        shown.contains("aMMM"),
        "the match's start and a little before: {shown}"
    );
    assert_accounts_for(&shown, &line);
}

#[test]
fn windows_are_cut_on_character_boundaries() {
    let line = format!("{}needle{}", "é".repeat(2000), "ü".repeat(2000));
    let at = "é".len() * 2000;
    for budget in [BUDGET, 7, 4, 1] {
        let (shown, _) = show(&line, &one(at..at + 6), budget);
        assert!(shown.contains("needle") || budget < 6, "{budget}: {shown}");
        let (kept, left_out) = accounted(&shown);
        assert_eq!(kept + left_out, line.len(), "{budget}: {shown}");
        assert!(kept <= budget, "{budget}: {shown}");
    }
}

#[test]
fn a_line_without_matches_is_shown_from_its_start() {
    let line = "z".repeat(600);
    let (shown, cut) = show(&line, &[], BUDGET);
    assert!(cut);
    assert_eq!(
        shown,
        format!("{}…[100 bytes]… [line is 600B]", "z".repeat(500))
    );
}

/// rg's offsets are into the bytes it read; a line decoded lossily (not
/// UTF-8) may be shorter, and a hit past it is dropped, not a panic.
#[test]
fn offsets_past_the_line_are_dropped() {
    let line = "q".repeat(600);
    let (shown, _) = show(
        &line,
        &[
            900..905,
            Range {
                start: 700,
                end: 650,
            },
        ],
        BUDGET,
    );
    assert!(
        shown.ends_with("[line is 600B]"),
        "no hit is claimed: {shown}"
    );
    assert_accounts_for(&shown, &line);
    let (shown, _) = show(&line, &one(550..700), BUDGET);
    assert!(
        shown.ends_with("; match at byte 550]"),
        "a hit is clamped to the line: {shown}"
    );
    assert_accounts_for(&shown, &line);
}

#[test]
fn an_empty_match_at_the_end_of_the_line_is_shown() {
    let line = "e".repeat(700);
    let (shown, _) = show(&line, &one(700..700), BUDGET);
    assert!(shown.starts_with("…[200 bytes]…"), "{shown}");
    assert!(shown.ends_with("; match at byte 700]"), "{shown}");
    assert!(!shown.contains("not shown"), "{shown}");
}

#[test]
fn many_hits_on_one_line_stay_within_the_budget() {
    let line = "a".repeat(100_000);
    let hits: Vec<_> = (0..100_000).map(|i| i..i + 1).collect();
    let (shown, _) = show(&line, &hits, BUDGET);
    assert!(
        shown.ends_with("; 100000 matches, first at byte 0; 99500 not shown]"),
        "{shown}"
    );
    assert_accounts_for(&shown, &line);
}

/// A line exactly the budget long fits: it is shown whole, not cut.
#[test]
fn a_line_exactly_the_budget_long_is_shown_whole() {
    let line = "w".repeat(BUDGET);
    for hits in [Vec::new(), one(BUDGET - 6..BUDGET)] {
        let (shown, cut) = show(&line, &hits, BUDGET);
        assert_eq!(shown, line);
        assert!(!cut);
    }
}

/// One byte past the window is still counted.
#[test]
fn a_single_byte_left_out_is_counted() {
    let line = "v".repeat(BUDGET + 1);
    let (shown, cut) = show(&line, &[], BUDGET);
    assert!(cut);
    assert_eq!(
        shown,
        format!("{}…[1 bytes]… [line is 501B]", "v".repeat(BUDGET))
    );
}

/// Hits are placed and located the same in whatever order they come.
#[test]
fn hits_in_any_order_are_shown_alike() {
    let line = "o".repeat(5000);
    let in_order = show(&line, &[100..106, 3000..3006], BUDGET);
    let reversed = show(&line, &[3000..3006, 100..106], BUDGET);
    assert_eq!(reversed, in_order);
    assert!(
        in_order.0.ends_with("; 2 matches, first at byte 100]"),
        "{}",
        in_order.0
    );
}

/// Matches closer than twice the context are one window, which the whole
/// budget widens (less the rounding of its two sides).
#[test]
fn close_matches_share_the_whole_budget() {
    let line = "c".repeat(2000);
    let (shown, _) = show(&line, &[1000..1005, 1050..1055], BUDGET);
    assert_eq!(shown.matches("…[").count(), 2, "one window: {shown}");
    let (kept, _) = accounted(&shown);
    assert!(kept >= BUDGET - 1, "the window is {kept} bytes: {shown}");
    assert_accounts_for(&shown, &line);
}

/// Matches too long for the budget near the line's end: the window ends
/// at the line's end and still shows the whole budget.
#[test]
fn a_match_too_long_for_the_budget_near_the_end_fills_the_budget() {
    let line = "t".repeat(1000);
    let (shown, _) = show(&line, &one(560..990), BUDGET);
    assert_eq!(
        shown,
        format!(
            "…[500 bytes]…{} [line is 1000B; match at byte 560]",
            "t".repeat(BUDGET)
        )
    );
}

/// Two windows that meet inside a character are one: the character is
/// shown, not counted as left out between them.
#[test]
fn windows_that_meet_inside_a_character_keep_it() {
    // Two one-byte hits 249 bytes apart: each window is 249 bytes wide,
    // the first ending where the second starts, at byte 1125, inside `é`.
    let line = format!("{}é{}", "x".repeat(1124), "y".repeat(874));
    let (shown, _) = show(&line, &[1000..1001, 1249..1250], BUDGET);
    assert!(shown.contains("xéy"), "{shown}");
    assert_eq!(shown.matches("…[").count(), 2, "one window: {shown}");
    assert_accounts_for(&shown, &line);
}

/// An empty match at a window's end (the line's end) is shown, so it is
/// not counted as hidden among several.
#[test]
fn an_empty_match_at_the_end_among_others_is_not_hidden() {
    let line = "h".repeat(2000);
    let (shown, _) = show(&line, &[100..101, 2000..2000], BUDGET);
    assert!(
        shown.ends_with("; 2 matches, first at byte 100]"),
        "{shown}"
    );
}

/// #2251 review (M2): on a line that is not UTF-8 the window is placed on
/// the match in the decoded text, and what the agent is told is in raw
/// bytes: 300 invalid bytes before the match, 300 between two.
#[test]
fn raw_offsets_on_a_line_that_is_not_utf8_find_their_matches() {
    let mut raw = vec![0xE9; 300];
    raw.extend_from_slice(b"first");
    raw.extend_from_slice(&[0xE9; 300]);
    raw.extend_from_slice(b"second");
    raw.extend_from_slice(&[b'a'; 2000]);
    let line = Decoded::new(&raw);
    let (shown, cut) = show_line(&line, &[300..305, 605..611], BUDGET);
    assert!(cut);
    assert!(shown.contains("\u{FFFD}first\u{FFFD}"), "{shown}");
    assert!(shown.contains("\u{FFFD}secondaa"), "{shown}");
    assert!(
        shown.ends_with("[line is 2.5KB; 2 matches, first at byte 300]"),
        "{shown}"
    );
}
