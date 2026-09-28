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
