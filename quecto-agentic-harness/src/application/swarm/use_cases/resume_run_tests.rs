use super::{MEMBERS_CANNOT_RESUME, ResumeRun};
use crate::domain::swarm::BoardError;

/// `resume_refuses_members_and_lists_blockers_for_the_supervisor` (the
/// members half): every member, the coordinator included, is refused with
/// the "outside the swarm" text.
#[test]
fn members_cannot_resume() {
    for actor in ["parent", "worker"] {
        assert_eq!(
            ResumeRun::new().execute(actor).unwrap_err(),
            BoardError::new(MEMBERS_CANNOT_RESUME)
        );
    }
    assert!(MEMBERS_CANNOT_RESUME.contains("outside the swarm"));
}
