//! The dialog's words (#2011 review R2-T3, R2-T4, R2-T7): for someone who has
//! never heard of a "workspace", a "record" or "linked"; a short form for a
//! narrow terminal; a clipped label says so.
use super::super::wording::{FOOTER, RowWording, titles};
use super::*;

const KINDS: [ResumeDecisionKind; 5] = [
    ResumeDecisionKind::CrossFolder,
    ResumeDecisionKind::HomeMissing,
    ResumeDecisionKind::HomeChanged,
    ResumeDecisionKind::HomeUnknown,
    ResumeDecisionKind::LegacyUnscoped,
];

fn offers() -> Vec<ResumeActionOffer> {
    vec![
        offer(ResumeAction::OpenOriginal, false, None),
        offer(ResumeAction::ForkCurrent, false, None),
        offer(ResumeAction::Locate, false, None),
        offer(ResumeAction::Associate, false, None),
        offer(ResumeAction::Cancel, true, None),
    ]
}

fn labels(width: usize) -> Vec<String> {
    let offers = offers();
    let items = RowWording::fitting(&offers, width).items(&offers, width);
    items.into_iter().map(|item| item.label).collect()
}

#[test]
fn every_kind_has_its_own_long_and_short_title_in_plain_words() {
    let all: std::collections::BTreeSet<_> = KINDS.into_iter().flat_map(titles).collect();
    assert_eq!(all.len(), 10);
    for [long, short] in KINDS.map(titles) {
        assert!(short.chars().count() <= 17, "{short}");
        for internal in ["workspace", "linked", "record", "legacy", "home"] {
            assert!(!long.contains(internal), "{internal:?} in {long:?}");
        }
    }
    // A folder that exists and cannot be read is not called missing (R2-T4).
    assert!(titles(ResumeDecisionKind::HomeMissing)[0].contains("no permission"));
    assert!(FOOTER.contains("your current session is untouched"));
    assert!(!FOOTER.contains("linked"));
}

#[test]
fn every_action_has_its_own_label_at_every_width() {
    for width in [200, 60, 40, 30, 24, 16] {
        let labels = labels(width);
        let distinct: std::collections::BTreeSet<_> = labels.iter().collect();
        assert_eq!(distinct.len(), 5, "{width}: {labels:?}");
    }
    assert_eq!(
        labels(200),
        [
            "Open original folder — unavailable",
            "Copy into this folder as a new session — unavailable",
            "Locate folder — unavailable",
            "Attach to a folder — unavailable",
            "Cancel",
        ]
    );
}

/// Long labels before short ones, a spelled mark before "(n/a)", the cross
/// last — and every unavailable row is marked at every width.
#[test]
fn the_widest_wording_that_fits_every_row_is_chosen() {
    assert_eq!(
        labels(46)[1],
        "Copy into this folder as a new session (n/a)"
    );
    assert_eq!(labels(45)[1], "Copy here — unavailable");
    assert_eq!(labels(29)[0], "Open original — unavailable");
    assert_eq!(labels(28)[0], "Open original (n/a)");
    assert_eq!(labels(20)[0], "✗ Open original");
    for width in [200, 46, 45, 29, 28, 21, 20, 17] {
        let labels = labels(width);
        for label in &labels[..4] {
            let marked = label.contains("unavailable") || label.contains("(n/a)");
            assert!(marked || label.starts_with('✗'), "{width}: {label:?}");
            assert!(visible_width(label) + 2 <= width, "{width}: {label:?}");
        }
        assert_eq!(labels[4], "Cancel");
    }
}

/// A label the width still clips ends in an ellipsis — never silently.
#[test]
fn a_clipped_label_is_marked_with_an_ellipsis() {
    let labels = labels(12);
    assert_eq!(labels[0], "✗ Open or…");
    assert_eq!(labels[1], "✗ Copy he…");
    for label in labels {
        assert!(visible_width(&label) <= 10, "{label:?}");
    }
}
