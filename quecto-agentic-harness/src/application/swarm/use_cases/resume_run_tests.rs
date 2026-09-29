use super::ResumeRun;
use crate::domain::swarm::{BoardError, RefusalKind};

/// `resume_refuses_members_and_lists_blockers_for_the_supervisor` (the
/// members half): every member, the coordinator included, is refused with
/// the "outside the swarm" text, as `supervisor_only`.
#[test]
fn members_cannot_resume() {
    for actor in ["parent", "worker"] {
        assert_eq!(
            ResumeRun::new().execute(actor).unwrap_err(),
            BoardError::new(
                RefusalKind::SupervisorOnly,
                "a paused run is resumed only by the supervisor outside the swarm \
                 (agent_cmd swarm_control resume); members cannot resume it"
            )
        );
    }
}
