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

fn folder(path: &str) -> SessionHome {
    SessionHome {
        execution_dir: path.into(),
        group: WorkspaceGroup::Folder {
            directory: path.into(),
        },
        provenance: AssociationProvenance::SavedHere,
    }
}

#[test]
fn admission_requires_unchanged_authority_in_the_current_directory() {
    let repo = SessionHome {
        execution_dir: "/repo".into(),
        group: WorkspaceGroup::Git {
            common_dir: "/repo/.git".into(),
        },
        provenance: AssociationProvenance::SavedHere,
    };
    assert_eq!(
        SessionHome::admission(&repo, &repo, &repo),
        HomeAdmission::Eligible
    );
    let mut linked = repo.clone();
    linked.execution_dir = "/linked".into();
    // Same group, different worktree: the execution-directory compare alone refuses.
    assert_eq!(
        SessionHome::admission(&repo, &repo, &linked),
        HomeAdmission::DifferentExecutionDirectory
    );
    assert_eq!(
        SessionHome::admission(&folder("/a"), &folder("/a"), &folder("/b")),
        HomeAdmission::DifferentExecutionDirectory
    );
}

#[test]
fn a_folder_that_became_a_repository_is_home_changed_not_a_different_directory() {
    let saved = folder("/repo");
    let now = SessionHome {
        execution_dir: "/repo".into(),
        group: WorkspaceGroup::Git {
            common_dir: "/repo/.git".into(),
        },
        provenance: AssociationProvenance::SavedHere,
    };
    assert_eq!(
        SessionHome::admission(&saved, &now, &now),
        HomeAdmission::HomeChanged
    );
    let mut moved = now.clone();
    moved.group = WorkspaceGroup::Git {
        common_dir: "/elsewhere/.git".into(),
    };
    assert_eq!(
        SessionHome::admission(&now, &moved, &moved),
        HomeAdmission::HomeChanged
    );
}
