//! Port vocabulary tests for resume runtime (#2001 D4).

use super::*;
use crate::domain::session_home_scope::CanonicalExecutionLocation;
use crate::domain::session_identity::SessionIdentity;

#[test]
fn open_original_launch_records_no_chdir_flag() {
    let launch = OpenOriginalLaunch {
        identity: SessionIdentity::from_persisted_key("chat-1"),
        runtime_root: CanonicalExecutionLocation::from_canonical_path("/orig"),
        launched_without_chdir: true,
    };
    assert!(launch.launched_without_chdir);
    assert_eq!(launch.runtime_root.as_str(), "/orig");
}

#[test]
fn fork_outcome_keeps_distinct_opaque_identities() {
    let outcome = ForkOutcome {
        source: SessionIdentity::from_persisted_key("chat-src"),
        new_identity: SessionIdentity::from_persisted_key("chat-new"),
        message_count: 3,
        new_home: crate::domain::session_home_scope::SessionHomeScope::scoped(
            CanonicalExecutionLocation::from_canonical_path("/cur"),
            None,
        ),
    };
    assert_ne!(outcome.source, outcome.new_identity);
    assert!(!outcome.new_identity.runtime_key().is_empty());
}
