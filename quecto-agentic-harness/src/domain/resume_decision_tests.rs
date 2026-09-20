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

fn id(key: &str) -> SessionIdentity {
    SessionIdentity::from_persisted_key(key)
}

fn of(scope: &SessionHomeScope) -> HomeVersion {
    HomeVersion::of(&id("cli:a"), scope)
}

/// Pinned literally (computed independently of this code): a token a client
/// holds must mean the same authority tomorrow, so no refactor — not even one
/// made symmetrically in the algorithm and a re-implementation — may move it.
#[test]
fn every_state_is_pinned_to_its_literal_token() {
    assert_eq!(of(&folder("/a")), of(&folder("/a")));
    for (scope, token) in [
        (SessionHomeScope::LegacyUnscoped, "h1-80bb5ff133c7b54b"),
        (folder("/a"), "h1-0dcc716908d9c85e"),
        (git("/a", "/a/.git"), "h1-28dc33537e6d8d17"),
        (
            SessionHomeScope::Unavailable("bad".into()),
            "h1-b597603abf079a65",
        ),
    ] {
        assert_eq!(of(&scope).as_str(), token, "{scope:?}");
    }
}

#[test]
fn every_authoritative_fact_changes_the_version() {
    let versions = [
        of(&SessionHomeScope::LegacyUnscoped),
        of(&SessionHomeScope::Unavailable("bad".into())),
        of(&folder("/a")),
        of(&folder("/b")),
        of(&git("/a", "/a/.git")),
        of(&git("/a", "/other/.git")),
        of(&git("/a/sub", "/a/.git")),
    ];
    let distinct: std::collections::BTreeSet<_> = versions.iter().cloned().collect();
    assert_eq!(distinct.len(), versions.len(), "{versions:?}");
}

/// The token of an uninterpretable home depends on the stable category, never
/// on the reader's error wording (review R2-H5): a serde or message change
/// between two harness versions must not turn a listed token stale — #2014
/// treats the token as a write authorization across an upgrade.
#[test]
fn an_uninterpretable_home_is_versioned_by_category_not_by_error_wording() {
    let token = of(&SessionHomeScope::Unavailable(
        "expected value at line 1 column 1".into(),
    ));
    for wording in [
        "expected value at line 1, column 1 (v2)",
        "unsupported home group",
        "",
    ] {
        assert_eq!(of(&SessionHomeScope::Unavailable(wording.into())), token);
    }
    assert_ne!(token, of(&SessionHomeScope::LegacyUnscoped));
}

/// #2014 authorizes a write with the token: it names one record. Two legacy
/// records, and two sessions saved in one folder, never share one.
#[test]
fn a_version_is_bound_to_the_session_identity() {
    for scope in [
        SessionHomeScope::LegacyUnscoped,
        SessionHomeScope::Unavailable("bad".into()),
        folder("/a"),
        git("/a", "/a/.git"),
    ] {
        assert_ne!(
            HomeVersion::of(&id("cli:a"), &scope),
            HomeVersion::of(&id("cli:b"), &scope),
            "{scope:?}"
        );
    }
}

/// Each pair concatenates to the same bytes when the length prefixes are
/// dropped — only the prefixes tell the fields apart.
#[test]
fn field_boundaries_are_unambiguous() {
    // exec "/x" + kind "git" + common "/ygit/z"  vs  exec "/xgit/y" + "git" + "/z"
    assert_ne!(of(&git("/x", "/ygit/z")), of(&git("/xgit/y", "/z")));
    // exec "/a" + "folder" + "/afolder/b"  vs  exec "/afolder/a" + "folder" + "/b"
    let folder_in = |exec: &str, directory: &str| {
        SessionHomeScope::Scoped(SessionHome {
            execution_dir: PathBuf::from(exec),
            group: WorkspaceGroup::Folder {
                directory: PathBuf::from(directory),
            },
            provenance: AssociationProvenance::SavedHere,
        })
    };
    assert_ne!(
        of(&folder_in("/a", "/afolder/b")),
        of(&folder_in("/afolder/a", "/b"))
    );
    // identity "cli:a" + "unavailable"  vs  identity "cli:aunavailable" + "unavailable"
    assert_ne!(
        HomeVersion::of(&id("cli:a"), &SessionHomeScope::Unavailable(String::new())),
        HomeVersion::of(
            &id("cli:aunavailable"),
            &SessionHomeScope::Unavailable(String::new())
        )
    );
}

#[test]
fn only_the_produced_shape_parses() {
    let version = of(&folder("/a"));
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
    assert_eq!(
        [
            CrossFolder,
            HomeMissing,
            HomeChanged,
            HomeUnknown,
            LegacyUnscoped
        ]
        .map(|kind| kind.resumes_by_opening_quecto_there()),
        [true, false, false, false, false],
        "only a session that lives in another folder is resumed by going there"
    );
    assert_eq!(
        [
            CrossFolder,
            HomeMissing,
            HomeChanged,
            HomeUnknown,
            LegacyUnscoped
        ]
        .map(|kind| kind.refusal_code()),
        [
            "belongs_elsewhere",
            "home_missing",
            "home_changed",
            "home_unknown",
            "no_home_recorded"
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
    assert!(LegacyUnscoped.to_string().contains("no folder recorded"));
}
