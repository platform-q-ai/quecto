use super::session_scope::{
    AssociationProvenance, CanonicalExecutionLocation, RepositoryGrouping, ResumeDisposition,
    SessionHomeScope, SessionScopeMetadata,
};

#[test]
fn scoped_home_keeps_execution_location_and_repository_facts_separate_from_identity() {
    let execution = CanonicalExecutionLocation::new("/work/repo").expect("absolute canonical path");
    let grouping = RepositoryGrouping::new("/work/repo/.git", "/work/repo/.git")
        .expect("non-empty git facts");
    let scope = SessionHomeScope::scoped(
        execution.clone(),
        Some(grouping.clone()),
        AssociationProvenance::Discovered,
    );

    assert_eq!(scope.execution_location(), Some(&execution));
    assert_eq!(scope.repository_grouping(), Some(&grouping));
    assert_eq!(scope.provenance(), Some(AssociationProvenance::Discovered));
}

#[test]
fn legacy_home_has_no_guessed_location_or_provenance() {
    let scope = SessionHomeScope::LegacyUnscoped;
    assert_eq!(scope.execution_location(), None);
    assert_eq!(scope.repository_grouping(), None);
    assert_eq!(scope.provenance(), None);
}

#[test]
fn canonical_location_and_repository_facts_reject_empty_values() {
    assert!(CanonicalExecutionLocation::new("").is_err());
    assert!(RepositoryGrouping::new("", "/repo/.git").is_err());
    assert!(RepositoryGrouping::new("/repo/.git", "").is_err());
}

#[test]
fn resume_dispositions_are_explicit_domain_choices() {
    let dispositions = [
        ResumeDisposition::SameScope,
        ResumeDisposition::OpenOriginal,
        ResumeDisposition::ForkCurrent,
        ResumeDisposition::Locate,
        ResumeDisposition::Cancel,
    ];
    assert_eq!(dispositions.len(), 5);
}

#[test]
fn metadata_is_versioned_and_round_trips_without_identity_data() {
    let metadata = SessionScopeMetadata::current(SessionHomeScope::LegacyUnscoped);
    let json = serde_json::to_string(&metadata).expect("serialize");
    assert!(json.contains("schemaVersion"));
    assert!(!json.contains("sessionKey"));
    let decoded: SessionScopeMetadata = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded, metadata);
}
