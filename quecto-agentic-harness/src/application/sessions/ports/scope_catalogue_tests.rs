//! Port-level vocabulary tests for the scope catalogue (#2001 D3).

use super::*;

#[test]
fn rebuild_report_defaults_are_explicit() {
    let r = CatalogueRebuildReport {
        rows_written: 3,
        rows_skipped: 1,
        rebuilt_from_scratch: true,
        notes: vec!["skipped corrupt entry".into()],
    };
    assert_eq!(r.rows_written, 3);
    assert!(r.rebuilt_from_scratch);
    assert_eq!(r.rows_skipped, 1);
}
