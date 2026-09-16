use super::*;
#[test]
fn grouping_does_not_authorize_different_execution_directory() {
    let home = SessionHome {
        execution_dir: "/repo".into(),
        group: WorkspaceGroup::Git {
            common_dir: "/repo/.git".into(),
        },
        provenance: AssociationProvenance::SavedHere,
    };
    let mut other = home.clone();
    other.execution_dir = "/linked".into();
    assert_eq!(home.group, other.group);
    assert!(!home.same_execution(&other));
    assert!(home.same_execution(&home));
}
