use super::super::start_fresh_rig::{FRESH_KEY, FreshOptions, OLD_KEY, build_fresh_rig};
use crate::application::sessions::dto::{
    FleetSettlementOutcome, SessionTransitionRefused, StartFreshConversationError,
};
use crate::domain::ids::AgentUuid;
use crate::domain::message::Message;

fn conversation(prompt: &str) -> Vec<Message> {
    let mut messages = vec![Message::user("q1"), Message::assistant("a1", vec![])];
    if !prompt.is_empty() {
        messages.insert(0, Message::system(prompt));
    }
    messages
}

/// The baseline effect order, step by step, with the fresh identity never
/// claimed: settle → save → replace roster → accounting/pending reset →
/// generate → release old → propagate → switch → effort → workflow →
/// clear the fresh namespace.
#[tokio::test]
async fn success_runs_every_baseline_step_in_order_and_never_claims_the_fresh_key() {
    let rig = build_fresh_rig(FreshOptions {
        prompt: "SYS",
        roster: Some((0, 2)),
        ..FreshOptions::default()
    });
    let mut messages = conversation("SYS");
    rig.record(&messages);
    rig.set_watermark(2);
    let epoch_before = rig.epoch();
    let fleet = rig.settled_fleet();
    let mut runtime = rig.runtime();

    let started = rig
        .fresh
        .execute(&mut messages, Some(&fleet), &mut runtime)
        .await
        .expect("started");

    assert_eq!(
        rig.journal(),
        [
            "fleet.settle",
            "store.save_clean_delta",
            "roster.clear",
            "accounting.reset(0)",
            "identity.generate(chat-1700000000-2a)",
            "store.release(cli:departing)",
            "key.propagate(chat-1700000000-2a)",
            "effort.reset",
            "workflow.reset",
            "retention.clear(chat-1700000000-2a)",
        ]
    );
    assert!(
        rig.store.claimed.lock().unwrap().is_empty(),
        "the fresh identity is never claimed"
    );
    assert_eq!(rig.store.released.lock().unwrap().as_slice(), [OLD_KEY]);
    assert_eq!(started.identity.runtime_key(), FRESH_KEY);
    assert_eq!(rig.identity(), FRESH_KEY);
    assert_eq!(started.ledger.epoch, epoch_before + 1);
    assert!(started.ledger.changed);
    // The conversation is cleared to its injected prompt; the saved copy
    // is the departing conversation, prompt stripped.
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "SYS");
    assert_eq!(rig.store.saved.lock().unwrap()[0].len(), 2);
    assert_eq!(rig.watermark(), 0);
    assert_eq!(runtime.resets, [0]);
    assert_eq!(runtime.propagated, [FRESH_KEY]);
    // Old refs stop resolving; the live transcript is the fresh session's.
    assert!(!rig.resolves(&Message::user("q1")));
    assert_eq!(rig.live_contents(), ["SYS"]);
    assert_eq!(
        rig.roster
            .unwrap()
            .records
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

#[tokio::test]
async fn an_unsettled_child_refuses_before_the_save_and_keeps_everything() {
    let rig = build_fresh_rig(FreshOptions {
        roster: Some((1, 1)),
        ..FreshOptions::default()
    });
    let mut messages = conversation("");
    rig.record(&messages);
    let fleet = rig.fleet(FleetSettlementOutcome::Unsettled(vec![(
        AgentUuid::new("A"),
        "ignores TERM".into(),
    )]));
    let mut runtime = rig.runtime();

    let err = rig
        .fresh
        .execute(&mut messages, Some(&fleet), &mut runtime)
        .await
        .expect_err("refused");

    assert!(matches!(
        &err,
        StartFreshConversationError::Refused(SessionTransitionRefused::Unsettled(children))
            if children.len() == 1
    ));
    assert_eq!(
        err.to_string(),
        "1 subagent(s) could not be settled; the current session was kept: A: ignores TERM"
    );
    assert_eq!(rig.journal(), ["fleet.settle"]);
    assert_eq!(messages.len(), 2, "nothing was cleared");
    assert_eq!(rig.identity(), OLD_KEY);
    assert!(rig.resolves(&messages[0]));
    assert!(rig.store.released.lock().unwrap().is_empty());
    assert!(runtime.propagated.is_empty());
}

#[tokio::test]
async fn an_interrupted_fleet_run_refuses_the_same_way() {
    let rig = build_fresh_rig(FreshOptions::default());
    let mut messages = conversation("");
    let fleet = rig.fleet(FleetSettlementOutcome::Interrupted);
    let err = rig
        .fresh
        .execute(&mut messages, Some(&fleet), &mut rig.runtime())
        .await
        .expect_err("refused");
    assert!(matches!(
        err,
        StartFreshConversationError::Refused(SessionTransitionRefused::Interrupted)
    ));
    assert_eq!(rig.journal(), ["fleet.settle"]);
    assert_eq!(messages.len(), 2);
    assert_eq!(rig.identity(), OLD_KEY);
}

#[tokio::test]
async fn without_a_fleet_live_delegated_rows_refuse_and_records_only_pass() {
    let rig = build_fresh_rig(FreshOptions {
        roster: Some((2, 2)),
        ..FreshOptions::default()
    });
    let mut messages = conversation("");
    let err = rig
        .fresh
        .execute(&mut messages, None, &mut rig.runtime())
        .await
        .expect_err("refused");
    assert!(matches!(
        err,
        StartFreshConversationError::Refused(SessionTransitionRefused::NoFleetTeardown(2))
    ));
    assert!(rig.journal().is_empty(), "nothing ran: {:?}", rig.journal());
    assert_eq!(messages.len(), 2);

    let rig = build_fresh_rig(FreshOptions {
        roster: Some((0, 3)),
        ..FreshOptions::default()
    });
    let mut messages = conversation("");
    rig.fresh
        .execute(&mut messages, None, &mut rig.runtime())
        .await
        .expect("records only");
    assert_eq!(
        &rig.journal()[..3],
        [
            "store.save_clean_delta",
            "roster.clear",
            "accounting.reset(0)"
        ]
    );
    assert_eq!(rig.identity(), FRESH_KEY);
}

/// The departing session's save fails after the fleet settled: the
/// failure is reported before any key replacement, the settled children
/// stay settled (the fleet ran), and the roster records, the key and the
/// conversation are kept.
#[tokio::test]
async fn a_failed_save_of_the_departing_session_refuses_before_any_key_replacement() {
    let rig = build_fresh_rig(FreshOptions {
        save_fails: true,
        roster: Some((0, 2)),
        ..FreshOptions::default()
    });
    let mut messages = conversation("");
    rig.record(&messages);
    let fleet = rig.settled_fleet();
    let mut runtime = rig.runtime();

    let err = rig
        .fresh
        .execute(&mut messages, Some(&fleet), &mut runtime)
        .await
        .expect_err("refused");

    assert!(matches!(err, StartFreshConversationError::Save(_)));
    assert_eq!(
        err.to_string(),
        "failed to save current session: session error: disk full"
    );
    assert_eq!(rig.journal(), ["fleet.settle", "store.save_clean_delta"]);
    assert_eq!(
        rig.roster
            .as_ref()
            .unwrap()
            .records
            .load(std::sync::atomic::Ordering::SeqCst),
        2,
        "the roster records were not replaced"
    );
    assert_eq!(messages.len(), 2);
    assert_eq!(rig.identity(), OLD_KEY);
    assert!(rig.resolves(&messages[1]));
    assert!(rig.store.released.lock().unwrap().is_empty());
    assert!(rig.store.claimed.lock().unwrap().is_empty());
    assert!(runtime.propagated.is_empty());
    assert!(runtime.resets.is_empty());
}

/// A live delegated row the fleet did not see (registered after the run)
/// refuses the roster replacement: the departing session was saved, but
/// nothing else moved and no row was dropped.
#[tokio::test]
async fn a_live_row_remaining_after_settlement_is_refused_not_dropped() {
    let rig = build_fresh_rig(FreshOptions {
        roster: Some((1, 1)),
        ..FreshOptions::default()
    });
    let mut messages = conversation("");
    let fleet = rig.settled_fleet();
    let mut runtime = rig.runtime();
    let err = rig
        .fresh
        .execute(&mut messages, Some(&fleet), &mut runtime)
        .await
        .expect_err("refused");
    assert!(matches!(
        err,
        StartFreshConversationError::Refused(SessionTransitionRefused::LiveRowsRemain(1))
    ));
    assert_eq!(
        err.to_string(),
        "1 live subagent(s) remain after the teardown; the current session was kept"
    );
    assert_eq!(rig.journal(), ["fleet.settle", "store.save_clean_delta"]);
    let roster = rig.roster.as_ref().unwrap();
    assert_eq!(roster.records.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(roster.live.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(messages.len(), 2);
    assert_eq!(rig.identity(), OLD_KEY);
    assert!(runtime.propagated.is_empty());
}

#[tokio::test]
async fn a_failed_clear_of_the_fresh_namespace_still_reports_success() {
    let rig = build_fresh_rig(FreshOptions {
        retention: Some(true),
        ..FreshOptions::default()
    });
    let mut messages = conversation("");
    let fleet = rig.settled_fleet();
    let started = rig
        .fresh
        .execute(&mut messages, Some(&fleet), &mut rig.runtime())
        .await
        .expect("success despite the retention failure");
    assert_eq!(started.identity.runtime_key(), FRESH_KEY);
    assert_eq!(
        rig.journal().last().map(String::as_str),
        Some("retention.clear(chat-1700000000-2a)")
    );
    assert!(messages.is_empty());
}

#[tokio::test]
async fn without_a_retention_store_or_a_roster_the_switch_still_completes() {
    let rig = build_fresh_rig(FreshOptions {
        retention: None,
        roster: None,
        ..FreshOptions::default()
    });
    let mut messages = conversation("");
    let started = rig
        .fresh
        .execute(&mut messages, None, &mut rig.runtime())
        .await
        .expect("started");
    assert_eq!(started.identity.runtime_key(), FRESH_KEY);
    assert_eq!(
        rig.journal(),
        [
            "store.save_clean_delta",
            "accounting.reset(0)",
            "identity.generate(chat-1700000000-2a)",
            "store.release(cli:departing)",
            "key.propagate(chat-1700000000-2a)",
            "effort.reset",
            "workflow.reset",
        ]
    );
}

/// An ephemeral loop saves nothing but still moves to a fresh chat key,
/// releasing the ephemeral identity exactly as before (the store ignores
/// it).
#[tokio::test]
async fn an_ephemeral_loop_skips_the_save_and_still_switches() {
    let rig = build_fresh_rig(FreshOptions {
        ephemeral: true,
        current_identity: "",
        ..FreshOptions::default()
    });
    let mut messages = conversation("");
    let started = rig
        .fresh
        .execute(&mut messages, None, &mut rig.runtime())
        .await
        .expect("started");
    assert_eq!(started.identity.runtime_key(), FRESH_KEY);
    assert!(rig.store.saved.lock().unwrap().is_empty());
    assert!(!rig.journal().iter().any(|e| e.starts_with("store.save")));
    assert_eq!(rig.store.released.lock().unwrap().as_slice(), [""]);
    assert_eq!(rig.identity(), FRESH_KEY);
    assert!(messages.is_empty());
}

#[tokio::test]
async fn the_old_ownership_is_released_only_when_the_identity_differs() {
    let rig = build_fresh_rig(FreshOptions {
        fresh_identity: OLD_KEY,
        ..FreshOptions::default()
    });
    let mut messages = conversation("");
    rig.fresh
        .execute(&mut messages, None, &mut rig.runtime())
        .await
        .expect("started");
    assert!(rig.store.released.lock().unwrap().is_empty());
    assert!(!rig.journal().iter().any(|e| e.starts_with("store.release")));
    assert_eq!(rig.identity(), OLD_KEY);
}

/// A killing exit armed for the departing session does not follow the
/// fresh one: the switch drops it.
#[tokio::test]
async fn a_killing_exit_armed_for_the_departing_session_is_dropped() {
    let rig = build_fresh_rig(FreshOptions::default());
    rig.state.write().await.set_killing_exit(true);
    let mut messages = conversation("");
    rig.fresh
        .execute(&mut messages, None, &mut rig.runtime())
        .await
        .expect("started");
    assert!(!rig.state.read().await.killing_exit());
}
