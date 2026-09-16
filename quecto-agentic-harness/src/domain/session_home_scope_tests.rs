//! D1 unit tests: folder-aware session home scope vocabulary (#2001).
//!
//! Pure domain only — no filesystem, Git, UI, or process calls.
//! Session keys remain opaque and never path-derived.

use super::*;
use crate::domain::session_identity::SessionIdentity;

// ─── SessionHomeScope ───────────────────────────────────────────────────────

#[test]
fn scoped_home_carries_execution_location_and_grouping_facts() {
    let location = CanonicalExecutionLocation::from_canonical_path("/work/repo");
    let grouping = RepositoryWorktreeGrouping::related_worktrees(
        RepositoryLabel::new("repo").expect("label"),
        CanonicalExecutionLocation::from_canonical_path("/work/repo"),
        vec![
            CanonicalExecutionLocation::from_canonical_path("/work/repo"),
            CanonicalExecutionLocation::from_canonical_path("/work/repo-wt2"),
        ],
    );
    let scope = SessionHomeScope::scoped(location.clone(), Some(grouping.clone()));
    match &scope {
        SessionHomeScope::Scoped {
            execution_location,
            repository_grouping,
        } => {
            assert_eq!(execution_location, &location);
            assert_eq!(repository_grouping.as_ref(), Some(&grouping));
            assert_eq!(execution_location.as_str(), "/work/repo");
        }
        SessionHomeScope::LegacyUnscoped => panic!("expected scoped home"),
    }
    assert!(scope.is_scoped());
    assert!(!scope.is_legacy_unscoped());
}

#[test]
fn legacy_unscoped_is_the_pre_scope_default() {
    let scope = SessionHomeScope::LegacyUnscoped;
    assert!(scope.is_legacy_unscoped());
    assert!(!scope.is_scoped());
    assert!(scope.execution_location().is_none());
    assert!(scope.repository_grouping().is_none());
}

#[test]
fn scoped_without_git_grouping_is_exact_folder_identity() {
    let location = CanonicalExecutionLocation::from_canonical_path("/tmp/notes");
    let scope = SessionHomeScope::scoped(location.clone(), None);
    assert_eq!(scope.execution_location(), Some(&location));
    assert!(scope.repository_grouping().is_none());
}

// ─── ResumeDisposition ──────────────────────────────────────────────────────

#[test]
fn resume_disposition_variants_cover_cross_folder_contract() {
    let all = [
        ResumeDisposition::SameScope,
        ResumeDisposition::OpenOriginal,
        ResumeDisposition::ForkCurrent,
        ResumeDisposition::Locate,
        ResumeDisposition::Cancel,
    ];
    assert_eq!(all.len(), 5);
    assert_eq!(ResumeDisposition::SameScope.as_str(), "same_scope");
    assert_eq!(ResumeDisposition::OpenOriginal.as_str(), "open_original");
    assert_eq!(ResumeDisposition::ForkCurrent.as_str(), "fork_current");
    assert_eq!(ResumeDisposition::Locate.as_str(), "locate");
    assert_eq!(ResumeDisposition::Cancel.as_str(), "cancel");
}

#[test]
fn resume_disposition_affirmative_predicates() {
    assert!(ResumeDisposition::SameScope.resumes_in_place());
    assert!(!ResumeDisposition::OpenOriginal.resumes_in_place());
    assert!(ResumeDisposition::OpenOriginal.launches_fresh_runtime());
    assert!(!ResumeDisposition::ForkCurrent.launches_fresh_runtime());
    assert!(ResumeDisposition::ForkCurrent.imports_transcript_only());
    assert!(ResumeDisposition::Locate.requires_explicit_reassociation());
    assert!(ResumeDisposition::Cancel.aborts_selection());
    assert!(!ResumeDisposition::SameScope.aborts_selection());
}

#[test]
fn resume_disposition_parses_allowlisted_wire_names_only() {
    assert_eq!(
        ResumeDisposition::parse("same_scope").unwrap(),
        ResumeDisposition::SameScope
    );
    assert_eq!(
        ResumeDisposition::parse("open_original").unwrap(),
        ResumeDisposition::OpenOriginal
    );
    assert_eq!(
        ResumeDisposition::parse("fork_current").unwrap(),
        ResumeDisposition::ForkCurrent
    );
    assert_eq!(
        ResumeDisposition::parse("locate").unwrap(),
        ResumeDisposition::Locate
    );
    assert_eq!(
        ResumeDisposition::parse("cancel").unwrap(),
        ResumeDisposition::Cancel
    );
    for rejected in ["", "chdir", "mkdir", "SAME_SCOPE", "open-original", "fork"] {
        assert!(
            ResumeDisposition::parse(rejected).is_err(),
            "must reject non-allowlisted disposition {rejected:?}"
        );
    }
}

// ─── CanonicalExecutionLocation ─────────────────────────────────────────────

#[test]
fn canonical_execution_location_is_exact_path_text_value() {
    let loc = CanonicalExecutionLocation::from_canonical_path("/home/u/proj");
    assert_eq!(loc.as_str(), "/home/u/proj");
    assert_eq!(loc.clone(), CanonicalExecutionLocation::from_canonical_path("/home/u/proj"));
    // Distinct path text is a distinct location (no silent normalisation here).
    assert_ne!(
        loc,
        CanonicalExecutionLocation::from_canonical_path("/home/u/proj/")
    );
}

#[test]
fn canonical_execution_location_rejects_empty_path() {
    let err = CanonicalExecutionLocation::try_from_canonical_path("").unwrap_err();
    assert!(
        err.to_string().contains("canonical execution location"),
        "{err}"
    );
}

// ─── Repository / worktree grouping facts ───────────────────────────────────

#[test]
fn repository_label_is_display_metadata_not_an_identity_fingerprint() {
    let label = RepositoryLabel::new("quecto").unwrap();
    assert_eq!(label.as_str(), "quecto");
    assert!(RepositoryLabel::new("").is_err());
    // Whitespace-only is not a usable label (affirmative non-empty allowlist).
    assert!(RepositoryLabel::new("   ").is_err());
}

#[test]
fn related_worktrees_keep_actual_execution_directory_distinguished() {
    let execution = CanonicalExecutionLocation::from_canonical_path("/wt/feature");
    let common = CanonicalExecutionLocation::from_canonical_path("/wt/main");
    let members = vec![
        common.clone(),
        execution.clone(),
        CanonicalExecutionLocation::from_canonical_path("/wt/hotfix"),
    ];
    let grouping = RepositoryWorktreeGrouping::related_worktrees(
        RepositoryLabel::new("quecto").unwrap(),
        execution.clone(),
        members.clone(),
    );
    assert_eq!(grouping.repository_label().as_str(), "quecto");
    assert_eq!(grouping.execution_location(), &execution);
    assert_eq!(grouping.member_locations(), members.as_slice());
    assert!(grouping.contains_location(&execution));
    assert!(grouping.contains_location(&common));
    assert!(!grouping.contains_location(&CanonicalExecutionLocation::from_canonical_path("/other")));
}

#[test]
fn single_worktree_grouping_still_records_execution_location() {
    let execution = CanonicalExecutionLocation::from_canonical_path("/only/repo");
    let grouping = RepositoryWorktreeGrouping::related_worktrees(
        RepositoryLabel::new("solo").unwrap(),
        execution.clone(),
        vec![execution.clone()],
    );
    assert_eq!(grouping.member_locations().len(), 1);
    assert_eq!(grouping.execution_location(), &execution);
}

// ─── Association provenance ─────────────────────────────────────────────────

#[test]
fn association_provenance_variants_are_explicit() {
    assert_eq!(
        AssociationProvenance::CreatedWithScope.as_str(),
        "created_with_scope"
    );
    assert_eq!(
        AssociationProvenance::ExplicitUserAssociation.as_str(),
        "explicit_user_association"
    );
    assert_eq!(
        AssociationProvenance::LocateReassociation.as_str(),
        "locate_reassociation"
    );
    assert_eq!(
        AssociationProvenance::ForkIntoCurrentScope.as_str(),
        "fork_into_current_scope"
    );
    for rejected in ["", "guessed", "auto", "implicit"] {
        assert!(AssociationProvenance::parse(rejected).is_err());
    }
    assert_eq!(
        AssociationProvenance::parse("explicit_user_association").unwrap(),
        AssociationProvenance::ExplicitUserAssociation
    );
}

#[test]
fn association_provenance_never_implies_guessed_legacy_binding() {
    assert!(!AssociationProvenance::CreatedWithScope.is_guessed());
    assert!(!AssociationProvenance::ExplicitUserAssociation.is_guessed());
    assert!(!AssociationProvenance::LocateReassociation.is_guessed());
    assert!(!AssociationProvenance::ForkIntoCurrentScope.is_guessed());
}

// ─── Versioned home-scope metadata (migration-compatible) ───────────────────

#[test]
fn home_scope_metadata_v1_round_trips_scoped_record() {
    let location = CanonicalExecutionLocation::from_canonical_path("/work/app");
    let grouping = RepositoryWorktreeGrouping::related_worktrees(
        RepositoryLabel::new("app").unwrap(),
        location.clone(),
        vec![location.clone()],
    );
    let meta = HomeScopeMetadata::v1_scoped(
        location.clone(),
        Some(grouping),
        AssociationProvenance::CreatedWithScope,
    );
    assert_eq!(meta.schema_version(), HOME_SCOPE_METADATA_VERSION_V1);
    assert_eq!(meta.scope(), &SessionHomeScope::scoped(location, Some(
        RepositoryWorktreeGrouping::related_worktrees(
            RepositoryLabel::new("app").unwrap(),
            CanonicalExecutionLocation::from_canonical_path("/work/app"),
            vec![CanonicalExecutionLocation::from_canonical_path("/work/app")],
        ),
    )));
    assert_eq!(meta.provenance(), AssociationProvenance::CreatedWithScope);
    assert!(!meta.is_legacy_unscoped());
}

#[test]
fn home_scope_metadata_v1_can_represent_legacy_unscoped() {
    let meta = HomeScopeMetadata::v1_legacy_unscoped();
    assert_eq!(meta.schema_version(), HOME_SCOPE_METADATA_VERSION_V1);
    assert_eq!(meta.scope(), &SessionHomeScope::LegacyUnscoped);
    assert!(meta.is_legacy_unscoped());
    // Legacy records have no association provenance until explicitly associated.
    assert!(meta.provenance_opt().is_none());
}

#[test]
fn home_scope_metadata_deserializes_missing_scope_as_legacy_readable() {
    // Old records without scope metadata remain readable as legacy-unscoped.
    let json = r#"{"schemaVersion":1}"#;
    let meta: HomeScopeMetadata = serde_json::from_str(json).expect("old-shaped metadata");
    assert!(meta.is_legacy_unscoped());
    assert_eq!(meta.scope(), &SessionHomeScope::LegacyUnscoped);
}

#[test]
fn home_scope_metadata_deserializes_empty_object_as_legacy_when_version_defaulted() {
    // Completely absent optional block is modeled by readers as legacy.
    let meta = HomeScopeMetadata::from_optional_json(None).expect("absent is ok");
    assert!(meta.is_legacy_unscoped());
}

#[test]
fn home_scope_metadata_rejects_unknown_future_version_without_guessing() {
    let json = r#"{"schemaVersion":99,"scope":"legacy_unscoped"}"#;
    let err = serde_json::from_str::<HomeScopeMetadata>(json).unwrap_err();
    assert!(
        err.to_string().contains("schema") || err.to_string().contains("version"),
        "{err}"
    );
}

#[test]
fn home_scope_metadata_serde_round_trip_preserves_scoped_fields() {
    let location = CanonicalExecutionLocation::from_canonical_path("/repo");
    let meta = HomeScopeMetadata::v1_scoped(
        location,
        None,
        AssociationProvenance::ExplicitUserAssociation,
    );
    let wire = serde_json::to_value(&meta).expect("serialize");
    assert_eq!(wire["schemaVersion"], 1);
    assert_eq!(wire["executionLocation"], "/repo");
    assert_eq!(wire["provenance"], "explicit_user_association");
    let back: HomeScopeMetadata = serde_json::from_value(wire).expect("deserialize");
    assert_eq!(back, meta);
}

// ─── Opaque session keys stay independent of home scope ─────────────────────

#[test]
fn home_scope_never_derives_or_rewrites_session_identity_keys() {
    let identity = SessionIdentity::from_persisted_key("chat-1700000000-abc");
    let location = CanonicalExecutionLocation::from_canonical_path("/path/that/must/not/become/key");
    let meta = HomeScopeMetadata::v1_scoped(
        location,
        None,
        AssociationProvenance::CreatedWithScope,
    );
    // Scope metadata is orthogonal: identity bytes unchanged.
    assert_eq!(identity.runtime_key(), "chat-1700000000-abc");
    assert_eq!(identity.persisted_key(), Some("chat-1700000000-abc"));
    assert!(!meta.scope().is_legacy_unscoped());
    // No API may path-derive a key — characterization of exact-key ownership.
    let same = SessionIdentity::from_persisted_key(identity.runtime_key());
    assert_eq!(same, identity);
}

#[test]
fn characterization_exact_key_categories_remain_opaque_beside_scope() {
    for key in [
        "cli:default",
        "chat-1700000000-1a2b",
        "telegram:12345",
        "weird key/with:slashes and spaces",
    ] {
        let identity = SessionIdentity::from_persisted_key(key);
        assert_eq!(identity.persisted_key(), Some(key));
        let scope = SessionHomeScope::LegacyUnscoped;
        assert!(scope.is_legacy_unscoped());
        // Associating scope later must not require rewriting the key.
        let associated = HomeScopeMetadata::v1_scoped(
            CanonicalExecutionLocation::from_canonical_path("/elsewhere"),
            None,
            AssociationProvenance::ExplicitUserAssociation,
        );
        assert_eq!(
            SessionIdentity::from_persisted_key(key).runtime_key(),
            key,
            "key must stay opaque after scope association vocabulary exists"
        );
        assert!(!associated.is_legacy_unscoped());
    }
}

#[test]
fn same_scope_vs_cross_scope_comparison_uses_location_equality() {
    let a = CanonicalExecutionLocation::from_canonical_path("/a");
    let b = CanonicalExecutionLocation::from_canonical_path("/b");
    let home_a = SessionHomeScope::scoped(a.clone(), None);
    let home_b = SessionHomeScope::scoped(b, None);
    assert!(home_a.matches_execution_location(&a));
    assert!(!home_a.matches_execution_location(
        &CanonicalExecutionLocation::from_canonical_path("/b")
    ));
    assert!(!home_a.same_scope_as(&home_b));
    assert!(home_a.same_scope_as(&SessionHomeScope::scoped(a, None)));
    // Legacy never silently matches a scoped home.
    assert!(!SessionHomeScope::LegacyUnscoped.same_scope_as(&home_a));
    assert!(!home_a.same_scope_as(&SessionHomeScope::LegacyUnscoped));
}
