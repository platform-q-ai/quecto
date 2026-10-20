//! Which git failures are a server refusing a filter.
use super::*;

#[test]
fn only_a_refused_filter_is_fetched_again_whole() {
    for (said, refused) in [
        ("fatal: filter 'tree' not supported", true),
        (
            "fatal: remote error: filter 'blob:limit' not supported",
            true,
        ),
        ("fatal: filter 'tree' could not be parsed", false),
        ("fatal: protocol 'ext' not supported", false),
        ("fatal: unable to access the remote", false),
    ] {
        assert_eq!(refused_filter(said), refused, "{said}");
    }
}

/// #2654: only git's own word that the tracking ref is locked, or moved
/// while it was locked, by another writer in this clone is such a race.
#[test]
fn only_a_lock_on_the_tracking_ref_is_a_local_race() {
    let tracking = "refs/remotes/origin/quecto/board";
    for (said, raced) in [
        (
            "error: cannot lock ref 'refs/remotes/origin/quecto/board': is at 0087cc but expected e2d505",
            true,
        ),
        (
            "error: cannot lock ref 'refs/remotes/origin/quecto/board': Unable to create '/r/.git/refs/remotes/origin/quecto/board.lock': File exists.",
            true,
        ),
        (
            "error: cannot lock ref 'refs/remotes/upstream/quecto/board': is at 0087cc but expected e2d505",
            false,
        ),
        (
            "error: cannot lock ref 'refs/remotes/origin/quecto/board': unable to resolve reference",
            false,
        ),
        ("fatal: unable to access the remote", false),
    ] {
        assert_eq!(locked_by_another_writer(said, tracking), raced, "{said}");
    }
}
