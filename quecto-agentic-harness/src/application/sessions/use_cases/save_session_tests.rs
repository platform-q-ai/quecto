//! Red tests of Save current session (#1860, D5 #1972) over port fakes:
//! ephemeral no-op, full save vs clean delta, shrink resetting the
//! watermark, the dirty prefix, restore-reason normalisation, killing-exit
//! precedence, workflow/roster snapshot, ordinal invariants, a store error
//! leaving the watermark and latch retryable, system prompt exclusion and
//! re-injection, cancellation, and the concurrent persistence barrier.
use futures::FutureExt;

use super::rig_tests::{
    RecordingStore, RigOptions, Write, build_rig, contents, dirty, ordinals, row, run,
    set_watermark, watermark,
};
use crate::application::sessions::dto::{SaveMode, SaveOutcome, SaveSessionError, SaveTrigger};
use crate::domain::error::DomainError;
use crate::domain::message::{Message, Role};
use crate::domain::session::SubagentRestoreReason;
use crate::domain::session_identity::SessionIdentity;

#[tokio::test]
async fn an_ephemeral_run_and_the_empty_key_are_affirmative_no_ops() {
    for options in [
        RigOptions {
            ephemeral: true,
            ..Default::default()
        },
        RigOptions {
            key: "",
            ..Default::default()
        },
    ] {
        let rig = build_rig(options);
        let mut messages = vec![Message::user("hello")];
        let outcome = rig
            .use_case
            .save(&mut messages, SaveTrigger::OrdinaryExit)
            .await
            .unwrap();
        assert_eq!(outcome, SaveOutcome::Ephemeral);
        let mut pending = Message::user("next");
        let outcome = rig
            .use_case
            .save_with_pending_prompt(&messages, &mut pending)
            .await
            .unwrap();
        assert_eq!(outcome, SaveOutcome::Ephemeral);
        assert!(rig.store.writes().is_empty(), "nothing reaches the store");
        assert_eq!(watermark(&rig), 0);
        assert_eq!(pending.ordinal, None, "no ordinal is minted for nothing");
    }
}

#[tokio::test]
async fn a_routine_save_with_a_clean_prefix_is_a_clean_delta_from_the_watermark() {
    let rig = build_rig(RigOptions::default());
    set_watermark(&rig, 1);
    let mut messages = vec![Message::user("old"), Message::assistant("new", vec![])];
    let outcome = rig
        .use_case
        .save(&mut messages, SaveTrigger::Routine)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        SaveOutcome::Saved {
            mode: SaveMode::CleanDelta,
            persisted: 2
        }
    );
    assert!(matches!(
        &rig.store.writes()[..],
        [Write::CleanDelta {
            previously_persisted: 1,
            messages,
            workflow_run: None
        }] if contents(messages) == ["old", "new"]
    ));
    assert_eq!(watermark(&rig), 2);
}

#[tokio::test]
async fn explicit_and_exit_triggers_force_a_full_save() {
    for trigger in [
        SaveTrigger::Explicit {
            restore_reason: SubagentRestoreReason::LegacyUnspecified,
        },
        SaveTrigger::OrdinaryExit,
    ] {
        let rig = build_rig(RigOptions::default());
        set_watermark(&rig, 1);
        let mut messages = vec![Message::user("old"), Message::assistant("new", vec![])];
        let outcome = rig.use_case.save(&mut messages, trigger).await.unwrap();
        assert_eq!(
            outcome,
            SaveOutcome::Saved {
                mode: SaveMode::Full,
                persisted: 2
            }
        );
        assert!(matches!(&rig.store.writes()[..], [Write::Full(session)]
            if session.key.runtime_key() == "cli:save"
                && contents(&session.messages) == ["old", "new"]
                && session.subagent_roster.is_empty()));
    }
}

#[tokio::test]
async fn a_tracked_roster_always_forces_a_full_save() {
    let rig = build_rig(RigOptions {
        roster: Some(vec![row("b"), row("a")]),
        ..Default::default()
    });
    set_watermark(&rig, 1);
    let mut messages = vec![Message::user("old"), Message::assistant("new", vec![])];
    rig.use_case
        .save(&mut messages, SaveTrigger::Routine)
        .await
        .unwrap();
    let [Write::Full(session)] = &rig.store.writes()[..] else {
        panic!(
            "a tracked roster is written whole: {:?}",
            rig.store.writes()
        );
    };
    let ids: Vec<_> = session
        .subagent_roster
        .iter()
        .map(|r| r.agent_uuid.as_str())
        .collect();
    assert_eq!(ids, ["a", "b"], "rows are ordered by identity");
    assert!(
        session
            .subagent_roster
            .iter()
            .all(|r| r.restore_reason == SubagentRestoreReason::LegacyUnspecified)
    );
}

#[tokio::test]
async fn a_shrunk_conversation_resets_the_watermark_before_the_store_is_asked() {
    let rig = build_rig(RigOptions::default());
    set_watermark(&rig, 5);
    let mut messages = vec![Message::user("only")];
    rig.use_case
        .save(&mut messages, SaveTrigger::Routine)
        .await
        .unwrap();
    assert!(matches!(
        &rig.store.writes()[..],
        [Write::CleanDelta {
            previously_persisted: 0,
            ..
        }]
    ));
    assert_eq!(watermark(&rig), 1);
}

#[tokio::test]
async fn a_dirty_prefix_replays_the_full_history_and_drains_the_latch() {
    let rig = build_rig(RigOptions::default());
    set_watermark(&rig, 2);
    rig.latch.latch();
    let mut messages = vec![Message::user("pruned-a"), Message::assistant("new", vec![])];
    let outcome = rig
        .use_case
        .save(&mut messages, SaveTrigger::Routine)
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        SaveOutcome::Saved {
            mode: SaveMode::Full,
            ..
        }
    ));
    assert!(!dirty(&rig), "a successful save drains the sticky latch");
    assert!(!rig.latch.take(), "the agent's latch was consumed");
    // The next routine save trusts the prefix again.
    messages.push(Message::user("later"));
    rig.use_case
        .save(&mut messages, SaveTrigger::Routine)
        .await
        .unwrap();
    assert!(matches!(
        rig.store.writes()[1],
        Write::CleanDelta {
            previously_persisted: 2,
            ..
        }
    ));
}

#[tokio::test]
async fn an_unknown_restore_reason_is_recorded_as_the_legacy_one() {
    let rig = build_rig(RigOptions {
        roster: Some(vec![row("child")]),
        ..Default::default()
    });
    let mut messages = vec![Message::user("hello")];
    rig.use_case
        .save(
            &mut messages,
            SaveTrigger::Explicit {
                restore_reason: SubagentRestoreReason::Unknown,
            },
        )
        .await
        .unwrap();
    let [Write::Full(session)] = &rig.store.writes()[..] else {
        panic!("{:?}", rig.store.writes());
    };
    assert_eq!(
        session.subagent_roster[0].restore_reason,
        SubagentRestoreReason::LegacyUnspecified
    );
    assert!(!rig.state.read().await.killing_exit());
}

#[tokio::test]
async fn an_explicitly_killed_reason_is_recorded_as_given() {
    let rig = build_rig(RigOptions {
        roster: Some(vec![row("child")]),
        ..Default::default()
    });
    let mut messages = vec![Message::user("hello")];
    rig.use_case
        .save(
            &mut messages,
            SaveTrigger::Explicit {
                restore_reason: SubagentRestoreReason::ExplicitlyKilled,
            },
        )
        .await
        .unwrap();
    let [Write::Full(session)] = &rig.store.writes()[..] else {
        panic!("{:?}", rig.store.writes());
    };
    assert_eq!(
        session.subagent_roster[0].restore_reason,
        SubagentRestoreReason::ExplicitlyKilled
    );
}

#[tokio::test]
async fn a_killing_exit_records_no_operational_child_and_survives_routine_saves() {
    let rig = build_rig(RigOptions {
        roster: Some(vec![row("live")]),
        ..Default::default()
    });
    let mut messages = vec![Message::user("keep transcript")];
    rig.use_case
        .save(
            &mut messages,
            SaveTrigger::Explicit {
                restore_reason: SubagentRestoreReason::OrdinaryTuiExitStopped,
            },
        )
        .await
        .unwrap();
    assert!(rig.state.read().await.killing_exit());
    rig.use_case
        .save(&mut messages, SaveTrigger::Routine)
        .await
        .unwrap();
    rig.use_case
        .save(&mut messages, SaveTrigger::OrdinaryExit)
        .await
        .unwrap();
    let writes = rig.store.writes();
    assert_eq!(writes.len(), 3);
    for write in &writes {
        let Write::Full(session) = write else {
            panic!("a killing exit keeps every save full: {write:?}");
        };
        assert!(session.subagent_roster.is_empty());
        assert_eq!(contents(&session.messages), ["keep transcript"]);
    }
}

#[tokio::test]
async fn an_explicit_detach_cancels_a_killing_exit_and_a_switch_drops_it() {
    let rig = build_rig(RigOptions {
        roster: Some(vec![row("live")]),
        ..Default::default()
    });
    let mut messages = vec![Message::user("hello")];
    for reason in [
        SubagentRestoreReason::OrdinaryTuiExitStopped,
        SubagentRestoreReason::LegacyUnspecified,
    ] {
        rig.use_case
            .save(
                &mut messages,
                SaveTrigger::Explicit {
                    restore_reason: reason,
                },
            )
            .await
            .unwrap();
    }
    assert!(!rig.state.read().await.killing_exit());
    let Write::Full(session) = &rig.store.writes()[1] else {
        panic!()
    };
    assert_eq!(session.subagent_roster.len(), 1);
    {
        let mut state = rig.state.write().await;
        state.set_killing_exit(true);
        state.switch_identity(SessionIdentity::from_persisted_key("cli:save"), None);
        assert!(state.killing_exit(), "the same identity keeps the intent");
        state.switch_identity(SessionIdentity::from_persisted_key("cli:other"), None);
        assert!(!state.killing_exit(), "a different identity drops it");
    }
}

#[tokio::test]
async fn the_workflow_run_and_the_roster_are_snapshotted_from_their_ports() {
    let rig = build_rig(RigOptions {
        workflow: Some(run()),
        roster: Some(vec![row("w")]),
        ..Default::default()
    });
    let mut messages = vec![Message::user("hello")];
    rig.use_case
        .save(&mut messages, SaveTrigger::Routine)
        .await
        .unwrap();
    let [Write::Full(session)] = &rig.store.writes()[..] else {
        panic!("{:?}", rig.store.writes());
    };
    assert_eq!(session.workflow_run, Some(run()));
    assert_eq!(session.subagent_roster[0].display_name, "worker-w");
    // Without a roster the workflow rides the clean delta.
    let rig = build_rig(RigOptions {
        workflow: Some(run()),
        ..Default::default()
    });
    rig.use_case
        .save(&mut messages, SaveTrigger::Routine)
        .await
        .unwrap();
    assert!(
        matches!(&rig.store.writes()[..], [Write::CleanDelta { workflow_run, .. }]
        if workflow_run.as_ref() == Some(&run()))
    );
}

#[tokio::test]
async fn ordinals_are_assigned_in_place_above_the_largest_existing_one() {
    let rig = build_rig(RigOptions::default());
    let mut old = Message::user("old");
    old.ordinal = Some(40);
    let mut messages = vec![old, Message::assistant("a", vec![]), Message::user("b")];
    rig.use_case
        .save(&mut messages, SaveTrigger::Routine)
        .await
        .unwrap();
    assert_eq!(ordinals(&messages), [Some(40), Some(41), Some(42)]);
    let Write::CleanDelta {
        messages: written, ..
    } = &rig.store.writes()[0]
    else {
        panic!()
    };
    assert_eq!(ordinals(written), [Some(40), Some(41), Some(42)]);
    // A later prune keeps the survivors' ordinals; new messages continue.
    messages.remove(0);
    messages.push(Message::user("c"));
    rig.use_case
        .save(&mut messages, SaveTrigger::Routine)
        .await
        .unwrap();
    assert_eq!(ordinals(&messages), [Some(41), Some(42), Some(43)]);
}

#[tokio::test]
async fn a_store_error_is_surfaced_verbatim_and_leaves_the_state_retryable() {
    let rig = build_rig(RigOptions::default());
    *rig.store.fail_with.lock().unwrap() = Some("disk full".into());
    set_watermark(&rig, 3);
    rig.latch.latch();
    let mut messages = vec![Message::user("a"), Message::user("b")];
    let err = rig
        .use_case
        .save(&mut messages, SaveTrigger::Routine)
        .await
        .expect_err("the store failed");
    assert!(
        matches!(&err, SaveSessionError::Store(DomainError::Session(text)) if text == "disk full")
    );
    assert_eq!(
        err.to_string(),
        DomainError::Session("disk full".into()).to_string()
    );
    assert_eq!(
        watermark(&rig),
        0,
        "the shrink reset stays: nothing is trusted"
    );
    assert!(
        dirty(&rig),
        "the drained latch stays sticky until a save succeeds"
    );
    assert!(
        !rig.latch.take(),
        "the agent's latch was moved, not left behind"
    );
    *rig.store.fail_with.lock().unwrap() = None;
    rig.use_case
        .save(&mut messages, SaveTrigger::Routine)
        .await
        .unwrap();
    assert!(
        matches!(rig.store.writes()[0], Write::Full(_)),
        "the retry replays"
    );
    assert_eq!(watermark(&rig), 2);
    assert!(!dirty(&rig));
}

#[tokio::test]
async fn the_injected_system_prompt_is_never_written_and_is_reinjected_after() {
    let rig = build_rig(RigOptions {
        prompt: "be helpful",
        ..Default::default()
    });
    let mut messages = vec![
        Message::system("be helpful\nwith extra"),
        Message::user("hello"),
    ];
    let outcome = rig
        .use_case
        .save(&mut messages, SaveTrigger::Routine)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        SaveOutcome::Saved {
            mode: SaveMode::CleanDelta,
            persisted: 1
        }
    );
    let Write::CleanDelta {
        messages: written, ..
    } = &rig.store.writes()[0]
    else {
        panic!()
    };
    assert_eq!(contents(written), ["hello"]);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, Role::System);
    assert_eq!(messages[0].content, "be helpful", "re-injected verbatim");
    assert_eq!(watermark(&rig), 1, "the watermark is a durable coordinate");
    // A failing store still re-injects the prompt.
    *rig.store.fail_with.lock().unwrap() = Some("read-only".into());
    let _ = rig.use_case.save(&mut messages, SaveTrigger::Routine).await;
    assert_eq!(messages[0].role, Role::System);
    // A real system message the user placed is not the injected one.
    let rig = build_rig(RigOptions {
        prompt: "be helpful",
        ..Default::default()
    });
    let mut messages = vec![Message::system("custom"), Message::user("hello")];
    rig.use_case
        .save(&mut messages, SaveTrigger::Routine)
        .await
        .unwrap();
    let Write::CleanDelta {
        messages: written, ..
    } = &rig.store.writes()[0]
    else {
        panic!()
    };
    assert_eq!(contents(written), ["custom", "hello"]);
    assert_eq!(messages.len(), 2, "nothing was injected over a real prompt");
}

#[tokio::test]
async fn the_pending_prompt_save_writes_a_verified_delta_and_stamps_the_ordinal() {
    let rig = build_rig(RigOptions {
        prompt: "be helpful",
        ..Default::default()
    });
    set_watermark(&rig, 1);
    let mut old = Message::user("old");
    old.ordinal = Some(9);
    let messages = vec![Message::system("be helpful"), old];
    let mut pending = Message::user("next");
    let outcome = rig
        .use_case
        .save_with_pending_prompt(&messages, &mut pending)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        SaveOutcome::Saved {
            mode: SaveMode::CleanDelta,
            persisted: 2
        }
    );
    assert_eq!(pending.ordinal, Some(10));
    assert!(matches!(
        &rig.store.writes()[..],
        [Write::Delta { previously_persisted: 1, messages, workflow_run: None }]
            if contents(messages) == ["old", "next"]
    ));
    assert_eq!(watermark(&rig), 2);
    assert_eq!(messages.len(), 2, "the live conversation is untouched");
    // With a tracked roster the pending save is a full write with legacy rows.
    let rig = build_rig(RigOptions {
        roster: Some(vec![row("w")]),
        ..Default::default()
    });
    rig.state.write().await.set_killing_exit(true);
    let mut pending = Message::user("next");
    rig.use_case
        .save_with_pending_prompt(&[Message::user("old")], &mut pending)
        .await
        .unwrap();
    let [Write::Full(session)] = &rig.store.writes()[..] else {
        panic!("{:?}", rig.store.writes());
    };
    assert_eq!(session.subagent_roster.len(), 1);
    assert_eq!(
        session.subagent_roster[0].restore_reason,
        SubagentRestoreReason::LegacyUnspecified
    );
    // A store error leaves the watermark and the pending ordinal as they were.
    let rig = build_rig(RigOptions::default());
    *rig.store.fail_with.lock().unwrap() = Some("busy".into());
    let mut pending = Message::user("next");
    let err = rig
        .use_case
        .save_with_pending_prompt(&[], &mut pending)
        .await
        .expect_err("store failed");
    assert_eq!(
        err.to_string(),
        DomainError::Session("busy".into()).to_string()
    );
    assert_eq!(watermark(&rig), 0);
}

#[tokio::test]
async fn a_cancelled_save_keeps_the_latch_and_the_watermark_retryable() {
    let rig = build_rig(RigOptions {
        store: RecordingStore::gated(),
        prompt: "be helpful",
        ..Default::default()
    });
    set_watermark(&rig, 1);
    rig.latch.latch();
    let mut messages = vec![Message::system("be helpful"), Message::user("a")];
    {
        let save = rig.use_case.save(&mut messages, SaveTrigger::Routine);
        assert!(
            save.now_or_never().is_none(),
            "the store blocks, so the save is pending and now dropped"
        );
    }
    assert_eq!(rig.store.started(), 1, "the store was reached");
    assert!(rig.store.writes().is_empty(), "nothing was recorded");
    assert_eq!(watermark(&rig), 1, "the watermark did not advance");
    assert!(dirty(&rig), "the sticky latch survives the cancellation");
    assert!(!rig.latch.take());
    assert_eq!(messages.len(), 1, "the prompt is stripped mid-save…");
    // …and the next save, from a fresh caller, re-injects it and replays.
    rig.store.gate.as_ref().unwrap().add_permits(1);
    rig.use_case
        .save(&mut messages, SaveTrigger::Routine)
        .await
        .unwrap();
    assert!(matches!(rig.store.writes()[0], Write::Full(_)));
    assert_eq!(messages[0].role, Role::System);
    assert_eq!(watermark(&rig), 1);
    assert!(!dirty(&rig));
}

#[tokio::test]
async fn concurrent_saves_are_serialised_by_the_barrier() {
    let rig = build_rig(RigOptions {
        store: RecordingStore::gated(),
        ..Default::default()
    });
    let first = {
        let use_case = rig.use_case.clone();
        tokio::spawn(async move {
            let mut messages = vec![Message::user("a"), Message::user("b")];
            use_case
                .save(&mut messages, SaveTrigger::Routine)
                .await
                .unwrap()
        })
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while rig.store.started() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the first save reaches the store");
    let mut second_messages = vec![Message::user("a"), Message::user("b"), Message::user("c")];
    let mut second = Box::pin(
        rig.use_case
            .save(&mut second_messages, SaveTrigger::Routine),
    );
    for _ in 0..8 {
        assert!(
            second.as_mut().now_or_never().is_none(),
            "the second save waits behind the first"
        );
        tokio::task::yield_now().await;
    }
    assert_eq!(
        rig.store.started(),
        1,
        "only the first save reached the store"
    );
    rig.store.gate.as_ref().unwrap().add_permits(2);
    let first_outcome = first.await.unwrap();
    let second_outcome = second.await.unwrap();
    assert_eq!(
        first_outcome,
        SaveOutcome::Saved {
            mode: SaveMode::CleanDelta,
            persisted: 2
        }
    );
    assert_eq!(
        second_outcome,
        SaveOutcome::Saved {
            mode: SaveMode::CleanDelta,
            persisted: 3
        }
    );
    let writes = rig.store.writes();
    assert!(matches!(
        writes[..],
        [
            Write::CleanDelta {
                previously_persisted: 0,
                ..
            },
            Write::CleanDelta {
                previously_persisted: 2,
                ..
            }
        ]
    ));
    assert_eq!(watermark(&rig), 3);
    assert!(format!("{:?}", rig.use_case).starts_with("SaveSession"));
}
