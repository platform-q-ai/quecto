//! Final report paging (#2285).

use super::*;

#[test]
fn a_long_report_is_paged_at_64_kib_on_character_boundaries() {
    let content = "é".repeat(FINAL_REPORT_PAGE_BYTES); // two bytes each
    let report = FinalReport {
        content: content.clone(),
        message_ordinal: None,
        failure: None,
    };
    let pages = report.pages();
    assert_eq!(pages.len(), 2);
    assert!(pages.iter().all(|p| p.len() <= FINAL_REPORT_PAGE_BYTES));
    assert_eq!(pages.concat(), content);
    let empty = FinalReport {
        content: String::new(),
        message_ordinal: None,
        failure: None,
    };
    assert_eq!(empty.pages(), vec![""]);
}

fn report(content: String) -> FinalReport {
    FinalReport {
        content,
        message_ordinal: None,
        failure: None,
    }
}

#[test]
fn the_page_budget_is_64_kib() {
    assert_eq!(FINAL_REPORT_PAGE_BYTES, 65_536);
}

#[test]
fn a_report_of_exactly_one_page_is_one_page() {
    let report = report("a".repeat(FINAL_REPORT_PAGE_BYTES));
    let pages = report.pages();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].len(), FINAL_REPORT_PAGE_BYTES);
}

#[test]
fn a_report_one_byte_over_a_page_is_a_full_page_and_one_byte() {
    let report = report("a".repeat(FINAL_REPORT_PAGE_BYTES + 1));
    let lengths: Vec<usize> = report.pages().iter().map(|p| p.len()).collect();
    assert_eq!(lengths, [FINAL_REPORT_PAGE_BYTES, 1]);
}

#[test]
fn a_char_straddling_a_page_end_starts_the_next_page() {
    // "a" then "é" at bytes 1-2, …, 65535-65536: the page end splits one.
    let content = format!("a{}", "é".repeat(FINAL_REPORT_PAGE_BYTES / 2));
    let report = report(content.clone());
    let pages = report.pages();
    let lengths: Vec<usize> = pages.iter().map(|p| p.len()).collect();
    assert_eq!(lengths, [FINAL_REPORT_PAGE_BYTES - 1, 2]);
    assert_eq!(pages[1], "é");
    assert_eq!(pages.concat(), content);
}

#[test]
fn a_four_byte_char_straddling_a_page_end_backs_off_three_bytes() {
    // "a" then "😀" at bytes 1-4, …, 65533-65536: byte 65536 is inside one.
    let content = format!("a{}", "😀".repeat(FINAL_REPORT_PAGE_BYTES / 4));
    let report = report(content.clone());
    let pages = report.pages();
    let lengths: Vec<usize> = pages.iter().map(|p| p.len()).collect();
    assert_eq!(lengths, [FINAL_REPORT_PAGE_BYTES - 3, 4]);
    assert_eq!(pages.concat(), content);
}
