//! Unit tests for cross-folder resume disposition DTOs (#2001 D4) — pure.

use super::*;
use crate::domain::session_home_scope::{
    AssociationProvenance, CanonicalExecutionLocation, ResumeDisposition, SessionHomeScope,
};
use crate::domain::session_identity::SessionIdentity;

fn scoped(path: &str) -> SessionHomeScope {
    SessionHomeScope::scoped(CanonicalExecutionLocation::from_canonical_path(path), None)
}

fn request(same: bool, reachable: bool) -> CrossFolderResumeRequest {
    let selected_home = scoped("/orig");
    let current_home = if same {
        scoped("/orig")
    } else {
        scoped("/current")
    };
    CrossFolderResumeRequest {
        selected: SessionIdentity::from_persisted_key("chat-sel"),
        selected_home,
        current_location: CanonicalExecutionLocation::from_canonical_path(if same {
            "/orig"
        } else {
            "/current"
        }),
        current_home,
        selected_home_reachable: reachable,
    }
}

#[test]
fn same_scope_request_plans_same_scope_resume() {
    let req = request(true, true);
    assert!(req.is_same_scope());
    assert!(!req.requires_cross_folder_choice());
    let plan = plan_disposition(&req, ResumeDisposition::SameScope).unwrap();
    assert_eq!(plan.disposition(), ResumeDisposition::SameScope);
    match plan {
        DispositionPlan::SameScopeResume { identity } => {
            assert_eq!(identity.runtime_key(), "chat-sel");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn cross_folder_same_scope_choice_is_not_applicable() {
    let req = request(false, true);
    assert!(req.requires_cross_folder_choice());
    let err = plan_disposition(&req, ResumeDisposition::SameScope).unwrap_err();
    assert!(matches!(
        err,
        DispositionError::DispositionNotApplicable {
            disposition: ResumeDisposition::SameScope
        }
    ));
}

#[test]
fn open_original_targets_selected_root_not_current() {
    let req = request(false, true);
    let plan = plan_disposition(&req, ResumeDisposition::OpenOriginal).unwrap();
    match plan {
        DispositionPlan::OpenOriginal {
            target_root,
            identity,
        } => {
            assert_eq!(target_root.as_str(), "/orig");
            assert_eq!(identity.runtime_key(), "chat-sel");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn open_original_refuses_when_selected_home_missing() {
    let req = request(false, false);
    assert!(req.selected_home_missing());
    let err = plan_disposition(&req, ResumeDisposition::OpenOriginal).unwrap_err();
    assert!(err.is_selected_home_missing());
}

#[test]
fn fork_and_locate_and_cancel_plan_shapes() {
    let req = request(false, true);
    let fork = plan_disposition(&req, ResumeDisposition::ForkCurrent).unwrap();
    assert_eq!(fork.disposition(), ResumeDisposition::ForkCurrent);
    let locate = plan_disposition(&req, ResumeDisposition::Locate).unwrap();
    assert_eq!(locate.disposition(), ResumeDisposition::Locate);
    match locate {
        DispositionPlan::LocateReassociate { provenance, .. } => {
            assert_eq!(
                provenance,
                AssociationProvenance::LocateReassociation
            );
        }
        other => panic!("{other:?}"),
    }
    let cancel = plan_disposition(&req, ResumeDisposition::Cancel).unwrap();
    assert!(!cancel.is_mutating());
}

#[test]
fn silent_cross_workspace_restore_is_refused() {
    let err = refuse_silent_cross_workspace_restore(&scoped("/a"), &scoped("/b")).unwrap_err();
    assert!(err.is_cross_workspace_refusal());
    refuse_silent_cross_workspace_restore(&scoped("/a"), &scoped("/a")).unwrap();
    refuse_silent_cross_workspace_restore(&SessionHomeScope::LegacyUnscoped, &scoped("/a")).unwrap();
}
