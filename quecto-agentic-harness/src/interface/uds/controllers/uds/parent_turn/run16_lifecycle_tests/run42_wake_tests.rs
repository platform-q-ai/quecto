//! Round3 wake boundaries through real pending delivery and workflow emissions.
use super::*;
use crate::domain::agents::value_objects::turn_outcome::TurnOutcome;
use crate::interface::cli::uds_session::{NoteClass, NotificationEnqueueOutcome, PendingMessage};

#[derive(Clone, Copy, Debug)]
enum Wake {
    Child,
    Workflow,
}

fn rig(wake: Wake, replies: Vec<Reply>) -> Rig {
    match wake {
        Wake::Child => Rig::new(replies),
        Wake::Workflow => Rig::composed(
            compose_with_config(
                true,
                |flags| {
                    flags.workflow_disabled = false;
                    flags.workflow = true;
                },
                workflow_config(),
            ),
            replies,
        ),
    }
}

fn fresh_information(rig: &mut Rig, wake: Wake) {
    match wake {
        Wake::Child => assert!(matches!(
            rig.env.session.enqueue_subagent_notification(
                "round3-child".into(),
                1,
                "round3-child finished with fresh information".into(),
                NoteClass::Completion,
            ),
            NotificationEnqueueOutcome::Retained
        )),
        Wake::Workflow => rig
            .env
            .workflow
            .lock()
            .unwrap()
            .select_template("run16", None)
            .unwrap(),
    }
}

async fn stalled_boundary(wake: Wake) {
    let mut replies = vec![Reply::Text("Unfinished"), Reply::Text("Unfinished")];
    replies.extend(done());
    replies.extend(done());
    let mut rig = rig(wake, replies);
    rig.prompt("round3-stalled").await;
    let initial = rig.frames();
    assert_eq!(rig.provider.requests().len(), 2);
    assert_eq!(of_type(&initial, "turn_outcome")[0]["outcome"], "stalled");
    let settled = rig.runtime.as_ref().unwrap().view().latest.clone();
    fresh_information(&mut rig, wake);
    rig.drain().await;
    let frames = rig.frames();
    let opt_in = std::env::var("QUECTO_TURN_OUTCOMES").ok().as_deref() == Some("1");
    match opt_in {
        true => {
            assert_eq!(
                rig.provider.requests().len(),
                2,
                "round3 exact-env1 STALLED must fence fresh {wake:?} ingress"
            );
            assert!(of_type(&frames, "turn_admitted").is_empty());
            assert!(of_type(&frames, "turn_outcome").is_empty());
            assert_eq!(rig.runtime.as_ref().unwrap().view().latest, settled);
            for _ in 0..3 {
                rig.drain().await;
            }
            assert_eq!(rig.provider.requests().len(), 2, "idle drains stay fenced");
        }
        false => {
            assert_eq!(rig.provider.requests().len(), 4, "default fresh wake");
            let admitted = of_type(&frames, "turn_admitted");
            assert_eq!(
                admitted.len(),
                1,
                "declaration call continues the same turn"
            );
            assert_ne!(admitted[0]["episodeId"], "round3-stalled");
            assert_eq!(of_type(&frames, "turn_outcome")[0]["outcome"], "done");
        }
    }
    rig.prompt("round3-user-rearm").await;
    let next = rig.frames();
    assert_eq!(
        of_type(&next, "turn_admitted")[0]["episodeId"],
        "round3-user-rearm"
    );
    assert_eq!(of_type(&next, "turn_outcome")[0]["outcome"], "done");
}

#[tokio::test]
async fn run42_round3_stalled_fresh_child_fenced_exact_env1() {
    if isolated(concat!(
        module_path!(),
        "::run42_round3_stalled_fresh_child_fenced_exact_env1"
    )) {
        stalled_boundary(Wake::Child).await;
    }
}

#[tokio::test]
async fn run42_round3_stalled_fresh_workflow_fenced_exact_env1() {
    if isolated(concat!(
        module_path!(),
        "::run42_round3_stalled_fresh_workflow_fenced_exact_env1"
    )) {
        stalled_boundary(Wake::Workflow).await;
    }
}

#[tokio::test]
async fn run42_round3_default_stalled_fresh_wakes_and_user_rearm_control() {
    if run21_default_isolated(concat!(
        module_path!(),
        "::run42_round3_default_stalled_fresh_wakes_and_user_rearm_control"
    )) {
        for wake in [Wake::Child, Wake::Workflow] {
            stalled_boundary(wake).await;
        }
    }
}

#[tokio::test]
async fn run42_round3_user_rearms_stalled_in_both_modes_control() {
    if isolated(concat!(
        module_path!(),
        "::run42_round3_user_rearms_stalled_in_both_modes_control"
    )) {
        let mut replies = vec![Reply::Text("Unfinished"), Reply::Text("Unfinished")];
        replies.extend(done());
        let mut rig = Rig::new(replies);
        rig.prompt("stalled-user").await;
        let first = rig.frames();
        assert_eq!(of_type(&first, "turn_outcome")[0]["outcome"], "stalled");
        rig.prompt("explicit-next-user").await;
        let next = rig.frames();
        assert_eq!(rig.provider.requests().len(), 4);
        assert_eq!(
            of_type(&next, "turn_admitted")[0]["episodeId"],
            "explicit-next-user"
        );
        assert_eq!(of_type(&next, "turn_outcome")[0]["outcome"], "done");
    }
}

/// The public runtime spends the correction; actual Automatic pending ingress
/// must select WorkWake, retain that episode and never grant another correction.
/// This is intentionally baseline-PASS coverage, not an invented RED.
#[tokio::test]
async fn run42_round3_workwake_ingress_preserves_open_episode_and_spent_budget() {
    if isolated(concat!(
        module_path!(),
        "::run42_round3_workwake_ingress_preserves_open_episode_and_spent_budget"
    )) {
        let mut rig = Rig::new(vec![Reply::Text("Wake received")]);
        let runtime = rig.runtime.as_ref().unwrap().clone();
        let first = runtime
            .admit(
                "cli:root",
                TurnAdmission::External {
                    episode_id: "round3-open".into(),
                },
                "first",
            )
            .unwrap();
        assert!(matches!(
            runtime.complete(first, TurnEnd::Completed(WorkLiveness::Empty)),
            ParentVerdict::Correct
        ));
        let correction = runtime
            .admit("cli:root", TurnAdmission::Correction, "correction")
            .unwrap();
        let waiting = match runtime.complete(correction, TurnEnd::Completed(WorkLiveness::Live)) {
            ParentVerdict::Published(record) => record,
            other => panic!("expected waiting, got {other:?}"),
        };
        assert_eq!(waiting.outcome, TurnOutcome::Waiting);
        rig.env
            .session
            .restore_pending(std::iter::once(PendingMessage::Automatic(
                "round3 actual WorkWake".into(),
            )));
        rig.drain().await;
        let frames = rig.frames();
        assert_eq!(rig.provider.requests().len(), 1, "no renewed correction");
        assert_eq!(
            rig.provider.requests()[0].last_user,
            "round3 actual WorkWake"
        );
        let admitted = of_type(&frames, "turn_admitted");
        assert_eq!(admitted.len(), 1);
        assert_eq!(admitted[0]["episodeId"], "round3-open");
        let outcomes = of_type(&frames, "turn_outcome");
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0]["outcome"], "stalled");
        assert_eq!(outcomes[0]["episodeId"], "round3-open");
        assert_eq!(outcomes[0]["sequence"], waiting.sequence + 1);
    }
}

#[tokio::test]
async fn run42_round3_live_open_work_accepts_fresh_child_control() {
    if isolated(concat!(
        module_path!(),
        "::run42_round3_live_open_work_accepts_fresh_child_control"
    )) {
        let mut rig = Rig::new(done());
        let runtime = rig.runtime.as_ref().unwrap().clone();
        let turn = runtime
            .admit(
                "cli:root",
                TurnAdmission::External {
                    episode_id: "live-open".into(),
                },
                "first",
            )
            .unwrap();
        let waiting = runtime.complete(turn, TurnEnd::Completed(WorkLiveness::Live));
        assert!(
            matches!(waiting, ParentVerdict::Published(ref record) if record.outcome == TurnOutcome::Waiting)
        );
        fresh_information(&mut rig, Wake::Child);
        rig.drain().await;
        let frames = rig.frames();
        assert_eq!(rig.provider.requests().len(), 2);
        assert_eq!(
            of_type(&frames, "turn_admitted")[0]["episodeId"],
            "live-open"
        );
        assert_eq!(of_type(&frames, "turn_outcome")[0]["outcome"], "done");
    }
}
