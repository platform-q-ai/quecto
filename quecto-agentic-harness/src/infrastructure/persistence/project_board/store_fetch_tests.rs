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
