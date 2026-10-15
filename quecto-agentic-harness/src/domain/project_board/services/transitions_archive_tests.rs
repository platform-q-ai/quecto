use super::*;
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::services::fixtures_tests::{
    at, open_run, person, task, without_claim,
};
use crate::domain::project_board::value_objects::vocabulary::RunOutcome;

/// A task in review whose run has ended is archived by anyone, whoever
/// held it and whether the claim is live, lapsed or gone: the claim is
/// cleared and the task is unresolved, as a cancelled one is. A run still
/// going holds it back for everyone, its holder included, so an archive
/// never races a live run.
#[test]
fn anyone_archives_a_reviewed_task_once_its_run_has_ended() {
    let mut ended = open_run(1, 1).into_fields();
    ended.ended = Some(at("09:30:00"));
    ended.outcome = Some(RunOutcome::Succeeded);
    let ended = Run::new(ended).unwrap();
    // Ada's claim runs 09:00 to 11:00: live at 10:00, lapsed hours ago
    // at 14:00, and absent once she let it go.
    for (reviewed, now) in [
        (task(Review), at("10:00:00")),
        (task(Review), at("14:00:00")),
        (without_claim(task(Review)), at("14:00:00")),
    ] {
        for actor in [person("Bob"), person("Ada")] {
            let archived = move_task(
                &reviewed,
                Archived,
                &actor,
                std::slice::from_ref(&ended),
                &[],
                &now,
            )
            .unwrap();
            let fields = archived.fields();
            assert_eq!(
                (fields.status, &fields.claim, fields.resolution, &fields.prs),
                (Archived, &None, None, &reviewed.fields().prs),
                "{actor:?} at {now:?}"
            );
        }
    }
    let running = open_run(1, 1);
    for actor in [person("Bob"), person("Ada")] {
        assert_eq!(
            move_task(
                &task(Review),
                Archived,
                &actor,
                std::slice::from_ref(&running),
                &[],
                &at("10:00:00")
            ),
            Err(BoardRuleError::RunOpen(1)),
            "{actor:?}: a run going holds the task"
        );
    }
}
