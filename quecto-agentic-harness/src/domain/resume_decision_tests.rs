use super::*;
use crate::domain::session_home::AssociationProvenance;
use std::path::PathBuf;

fn folder(dir: &str) -> SessionHomeScope {
    SessionHomeScope::Scoped(SessionHome {
        execution_dir: PathBuf::from(dir),
        group: WorkspaceGroup::Folder {
            directory: PathBuf::from(dir),
        },
        provenance: AssociationProvenance::SavedHere,
    })
}

fn git(dir: &str, common: &str) -> SessionHomeScope {
    SessionHomeScope::Scoped(SessionHome {
        execution_dir: PathBuf::from(dir),
        group: WorkspaceGroup::Git {
            common_dir: PathBuf::from(common),
        },
        provenance: AssociationProvenance::SavedHere,
    })
}

#[test]
fn a_version_is_deterministic_and_pinned_across_releases() {
    assert_eq!(
        HomeVersion::of(&folder("/a")),
        HomeVersion::of(&folder("/a"))
    );
    // Pinned: a token a client holds must mean the same authority tomorrow.
    assert_eq!(
        HomeVersion::of(&SessionHomeScope::LegacyUnscoped).as_str(),
        format!("h1-{}", legacy_digest())
    );
}

fn legacy_digest() -> String {
    // FNV-1a of the 8-byte little-endian length 6 followed by "legacy".
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in 6u64.to_le_bytes().iter().chain(b"legacy") {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[test]
fn every_authoritative_fact_changes_the_version() {
    let versions = [
        HomeVersion::of(&SessionHomeScope::LegacyUnscoped),
        HomeVersion::of(&SessionHomeScope::Unavailable("bad".into())),
        HomeVersion::of(&SessionHomeScope::Unavailable("worse".into())),
        HomeVersion::of(&folder("/a")),
        HomeVersion::of(&folder("/b")),
        HomeVersion::of(&git("/a", "/a/.git")),
        HomeVersion::of(&git("/a", "/other/.git")),
        HomeVersion::of(&git("/a/sub", "/a/.git")),
    ];
    let distinct: std::collections::BTreeSet<_> = versions.iter().cloned().collect();
    assert_eq!(distinct.len(), versions.len(), "{versions:?}");
}

#[test]
fn field_boundaries_are_unambiguous() {
    assert_ne!(
        HomeVersion::of(&git("/ab", "/c")),
        HomeVersion::of(&git("/a", "b/c"))
    );
}

#[test]
fn only_the_produced_shape_parses() {
    let version = HomeVersion::of(&folder("/a"));
    assert_eq!(HomeVersion::parse(version.as_str()), Some(version));
    for hostile in [
        "",
        "h1-",
        "h2-0123456789abcdef",
        "h1-0123456789ABCDEF",
        "h1-0123456789abcde",
        "h1-0123456789abcdef0",
        "h1-0123456789abcdeg",
        " h1-0123456789abcdef",
        "h1-0123456789abcde\u{1b}",
    ] {
        assert_eq!(HomeVersion::parse(hostile), None, "{hostile:?}");
    }
}

#[test]
fn action_names_round_trip_and_nothing_else_is_an_action() {
    for action in ResumeAction::ALL {
        assert_eq!(ResumeAction::from_name(action.name()), Some(action));
    }
    for hostile in ["", "restore", "Cancel", "cancel ", "open-original"] {
        assert_eq!(ResumeAction::from_name(hostile), None, "{hostile:?}");
    }
}

#[test]
fn every_kind_offers_exactly_its_contracted_actions_ending_in_cancel() {
    use ResumeAction::{Associate, Cancel, ForkCurrent, Locate, OpenOriginal};
    use ResumeDecisionKind::{CrossFolder, HomeChanged, HomeMissing, HomeUnknown, LegacyUnscoped};
    assert_eq!(
        CrossFolder.offered_actions(),
        [OpenOriginal, ForkCurrent, Cancel]
    );
    for kind in [HomeMissing, HomeChanged, HomeUnknown] {
        assert_eq!(kind.offered_actions(), [Locate, ForkCurrent, Cancel]);
    }
    assert_eq!(LegacyUnscoped.offered_actions(), [Associate, Cancel]);
    assert!(CrossFolder.offers(OpenOriginal) && !CrossFolder.offers(Locate));
    assert!(!LegacyUnscoped.offers(ForkCurrent) && !LegacyUnscoped.offers(OpenOriginal));
}

#[test]
fn kinds_have_stable_names_and_a_readable_reason() {
    use ResumeDecisionKind::{CrossFolder, HomeChanged, HomeMissing, HomeUnknown, LegacyUnscoped};
    let names: Vec<_> = [
        CrossFolder,
        HomeMissing,
        HomeChanged,
        HomeUnknown,
        LegacyUnscoped,
    ]
    .iter()
    .map(|kind| kind.name())
    .collect();
    assert_eq!(
        names,
        [
            "cross_folder",
            "home_missing",
            "home_changed",
            "home_unknown",
            "legacy_unscoped"
        ]
    );
    assert!(
        CrossFolder
            .to_string()
            .contains("different execution directory")
    );
    assert!(HomeMissing.to_string().contains("missing"));
    assert!(HomeChanged.to_string().contains("workspace changed"));
    assert!(HomeUnknown.to_string().contains("cannot be interpreted"));
    assert!(
        LegacyUnscoped
            .to_string()
            .contains("explicit first association")
    );
}
