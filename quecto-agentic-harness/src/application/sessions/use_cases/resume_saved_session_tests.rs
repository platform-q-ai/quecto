//! Resume saved session (D8 #1977) over the journaling rig: the exact
//! effect order of a success, every refusal before any mutation, the
//! target claim released on each failure after it, the old key released
//! immediately after the active key is replaced, and the startup open.
use std::sync::atomic::Ordering;

use super::super::start_fresh_rig::{FreshOptions, FreshRig, OLD_KEY, build_fresh_rig};
use crate::application::sessions::dto::{
    FleetSettlementOutcome, ResumeSavedSessionError, SessionTransitionRefused,
};
use crate::domain::ids::AgentUuid;
use crate::domain::message::Message;
use crate::domain::session::{
    PersistedSubagentRosterEntry, Session, SubagentLiveness, SubagentRestoreReason,
};
use crate::domain::session_identity::SessionIdentity;
use crate::domain::workflow::WorkflowRunPersisted;

const TARGET: &str = "cli:saved";

fn conversation(prompt: &str) -> Vec<Message> {
    let mut messages = vec![Message::user("q1"), Message::assistant("a1", vec![])];
    if !prompt.is_empty() {
        messages.insert(0, Message::system(prompt));
    }
    messages
}

fn saved_run() -> WorkflowRunPersisted {
    WorkflowRunPersisted {
        template_id: Some("feature".into()),
        done: vec![true, false],
        active_issue: None,
    }
}

fn history_row(id: &str) -> PersistedSubagentRosterEntry {
    PersistedSubagentRosterEntry {
        agent_uuid: id.to_string(),
        display_name: format!("worker-{id}"),
        session_key: id.to_string(),
        liveness: SubagentLiveness::Live,
        restore_reason: SubagentRestoreReason::LegacyUnspecified,
        parent_id: Some("parent".to_string()),
        read_only: true,
        delivered_message_ordinal: None,
        pending_message_reports: std::collections::VecDeque::new(),
        status: Some("idle".to_string()),
    }
}

/// A rig whose store holds `TARGET` with three messages, a workflow run
/// and two persisted (history-only) child rows.
fn rig_with_target(options: FreshOptions) -> FreshRig {
    let rig = build_fresh_rig(options);
    rig.store.seed(Session {
        key: SessionIdentity::from_persisted_key(TARGET),
        messages: vec![
            Message::user("saved-1"),
            Message::assistant("saved-2", vec![]),
            Message::user("saved-3"),
        ],
        workflow_run: Some(saved_run()),
        subagent_roster: vec![history_row("h1"), history_row("h2")],
    });
    rig
}

/// The baseline effect order, step by step: settle → save → claim target →
/// load → replace roster → release old (after the commit point, while the
/// active session still stands for the departing identity) → propagate →
/// effort → workflow restore → accounting reset → switch. No retention
/// clear: the resumed namespace keeps its entries.
#[tokio::test]
async fn success_runs_every_baseline_step_in_order_and_restores_history_and_workflow() {
    let rig = rig_with_target(FreshOptions {
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

    let resumed = rig
        .resume
        .execute("saved", &mut messages, Some(&fleet), &mut runtime)
        .await
        .expect("resumed");

    assert_eq!(
        rig.journal(),
        [
            "fleet.settle",
            "store.save_clean_delta",
            "store.claim(cli:saved)",
            "store.load(cli:saved)",
            "roster.clear",
            "store.release(cli:departing)@active=cli:departing",
            "key.propagate(cli:saved)@active=cli:departing",
            "effort.reset",
            "workflow.restore(feature)",
            "accounting.reset(3)",
        ]
    );
    assert_eq!(rig.store.claimed.lock().unwrap().as_slice(), [TARGET]);
    assert_eq!(rig.store.released.lock().unwrap().as_slice(), [OLD_KEY]);
    // Target claim and active identity cannot diverge after success.
    assert_eq!(resumed.identity.runtime_key(), TARGET);
    assert_eq!(rig.identity(), TARGET);
    assert_eq!(resumed.name, "saved");
    assert_eq!(resumed.ledger.epoch, epoch_before + 1);
    assert!(resumed.ledger.changed);
    // The live conversation is the loaded history behind the re-injected
    // prompt; the acknowledgement counts the prompt (#1534).
    assert_eq!(
        messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>(),
        ["SYS", "saved-1", "saved-2", "saved-3"]
    );
    assert_eq!(resumed.message_count, 4);
    assert_eq!(rig.watermark(), 3, "what the store holds is durable");
    assert_eq!(runtime.resets, [3]);
    assert_eq!(runtime.propagated, [TARGET]);
    // The departing conversation was saved, prompt stripped.
    assert_eq!(rig.store.saved.lock().unwrap()[0].len(), 2);
    // Old refs stop resolving; the live transcript is the resumed one.
    assert!(!rig.resolves(&Message::user("q1")));
    assert_eq!(
        rig.live_contents(),
        ["SYS", "saved-1", "saved-2", "saved-3"]
    );
    // #1937: the persisted rows created no operational row.
    let roster = rig.roster.unwrap();
    assert_eq!(roster.records.load(Ordering::SeqCst), 0);
    assert_eq!(roster.live.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_target_without_a_saved_workflow_resets_the_engine() {
    let rig = build_fresh_rig(FreshOptions::default());
    rig.store.seed(Session {
        key: SessionIdentity::from_persisted_key(TARGET),
        messages: vec![Message::user("only")],
        workflow_run: None,
        subagent_roster: Vec::new(),
    });
    let mut messages = conversation("");
    let mut runtime = rig.runtime();
    rig.resume
        .execute("saved", &mut messages, None, &mut runtime)
        .await
        .expect("resumed");
    assert!(rig.journal().contains(&"workflow.reset".to_string()));
    assert!(
        !rig.journal()
            .iter()
            .any(|e| e.starts_with("workflow.restore"))
    );
    assert_eq!(runtime.resets, [1]);
}

#[tokio::test]
async fn an_ephemeral_loop_refuses_before_touching_anything() {
    let rig = rig_with_target(FreshOptions {
        ephemeral: true,
        ..FreshOptions::default()
    });
    let mut messages = conversation("");
    let mut runtime = rig.runtime();
    let err = rig
        .resume
        .execute(
            "saved",
            &mut messages,
            Some(&rig.settled_fleet()),
            &mut runtime,
        )
        .await
        .expect_err("refused");
    assert!(matches!(err, ResumeSavedSessionError::Ephemeral));
    assert_eq!(err.to_string(), "cannot resume sessions in ephemeral mode");
    assert!(rig.journal().is_empty(), "nothing ran");
    assert_eq!(messages.len(), 2);
}

#[tokio::test]
async fn an_invalid_name_refuses_before_the_fleet_runs() {
    let rig = rig_with_target(FreshOptions::default());
    let mut messages = conversation("");
    let mut runtime = rig.runtime();
    let err = rig
        .resume
        .execute(
            "bad name!",
            &mut messages,
            Some(&rig.settled_fleet()),
            &mut runtime,
        )
        .await
        .expect_err("refused");
    assert!(matches!(err, ResumeSavedSessionError::InvalidName));
    assert!(rig.journal().is_empty(), "no settlement, no save, no claim");
    assert_eq!(rig.identity(), OLD_KEY);
}

#[tokio::test]
async fn an_unsettled_child_refuses_before_the_save_and_the_claim() {
    let rig = rig_with_target(FreshOptions {
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
        .resume
        .execute("saved", &mut messages, Some(&fleet), &mut runtime)
        .await
        .expect_err("refused");
    assert!(matches!(
        err,
        ResumeSavedSessionError::Refused(SessionTransitionRefused::Unsettled(_))
    ));
    assert_eq!(rig.journal(), ["fleet.settle"]);
    assert!(rig.store.claimed.lock().unwrap().is_empty());
    assert_eq!(rig.identity(), OLD_KEY);
    assert!(rig.resolves(&messages[0]));
}

#[tokio::test]
async fn a_failed_save_refuses_before_the_claim_with_the_fleet_already_run() {
    let rig = rig_with_target(FreshOptions {
        save_fails: true,
        ..FreshOptions::default()
    });
    let mut messages = conversation("");
    let mut runtime = rig.runtime();
    let err = rig
        .resume
        .execute(
            "saved",
            &mut messages,
            Some(&rig.settled_fleet()),
            &mut runtime,
        )
        .await
        .expect_err("refused");
    assert!(matches!(err, ResumeSavedSessionError::Save(_)));
    assert_eq!(
        err.to_string(),
        "failed to save current session: session error: disk full"
    );
    assert_eq!(rig.journal(), ["fleet.settle", "store.save_clean_delta"]);
    assert!(rig.store.claimed.lock().unwrap().is_empty());
    assert!(rig.store.released.lock().unwrap().is_empty());
    assert_eq!(rig.identity(), OLD_KEY);
}

#[tokio::test]
async fn a_target_owned_elsewhere_is_refused_at_the_claim_and_nothing_is_released() {
    let rig = rig_with_target(FreshOptions::default());
    *rig.store.owned_elsewhere.lock().unwrap() = Some(TARGET.into());
    let mut messages = conversation("");
    let mut runtime = rig.runtime();
    let err = rig
        .resume
        .execute("saved", &mut messages, None, &mut runtime)
        .await
        .expect_err("refused");
    assert!(matches!(err, ResumeSavedSessionError::Claim(_)));
    assert_eq!(
        err.to_string(),
        "session error: session cli:saved is owned by another live process"
    );
    assert_eq!(
        rig.journal(),
        ["store.save_clean_delta", "store.claim(cli:saved)"]
    );
    assert!(
        rig.store.released.lock().unwrap().is_empty(),
        "a refused claim leaves the other owner's claim and ours alone"
    );
    assert_eq!(rig.identity(), OLD_KEY);
    assert!(runtime.propagated.is_empty());
}

#[tokio::test]
async fn a_missing_target_releases_the_claim_just_taken_and_keeps_the_session() {
    let rig = build_fresh_rig(FreshOptions::default());
    let mut messages = conversation("");
    rig.record(&messages);
    let mut runtime = rig.runtime();
    let err = rig
        .resume
        .execute("cli:gone", &mut messages, None, &mut runtime)
        .await
        .expect_err("refused");
    assert!(matches!(err, ResumeSavedSessionError::NotFound(_)));
    assert_eq!(err.to_string(), "session not found: cli:gone");
    assert_eq!(
        rig.journal(),
        [
            "store.save_clean_delta",
            "store.claim(cli:gone)",
            "store.load(cli:gone)",
            "store.release(cli:gone)@active=cli:departing",
        ]
    );
    assert_eq!(rig.identity(), OLD_KEY);
    assert_eq!(messages.len(), 2);
    assert!(rig.resolves(&messages[0]));
    assert!(runtime.propagated.is_empty());
}

#[tokio::test]
async fn a_load_error_releases_the_claim_just_taken_and_keeps_the_session() {
    let rig = rig_with_target(FreshOptions::default());
    rig.store.fail_load.store(true, Ordering::SeqCst);
    let mut messages = conversation("");
    let mut runtime = rig.runtime();
    let err = rig
        .resume
        .execute("saved", &mut messages, None, &mut runtime)
        .await
        .expect_err("refused");
    assert!(matches!(err, ResumeSavedSessionError::Load(_)));
    assert_eq!(
        err.to_string(),
        "failed to load session: session error: corrupt session file"
    );
    assert_eq!(rig.store.released.lock().unwrap().as_slice(), [TARGET]);
    assert_eq!(rig.identity(), OLD_KEY);
    assert_eq!(messages.len(), 2);
}

#[tokio::test]
async fn live_rows_after_settlement_release_the_claim_and_keep_the_roster() {
    // The fleet reports settled, yet a live delegated row remains (a
    // registration the run did not see): refused after the load, the
    // target claim released, no row dropped.
    let rig = rig_with_target(FreshOptions {
        roster: Some((1, 3)),
        ..FreshOptions::default()
    });
    let mut messages = conversation("");
    let mut runtime = rig.runtime();
    let err = rig
        .resume
        .execute(
            "saved",
            &mut messages,
            Some(&rig.settled_fleet()),
            &mut runtime,
        )
        .await
        .expect_err("refused");
    assert!(matches!(
        err,
        ResumeSavedSessionError::Refused(SessionTransitionRefused::LiveRowsRemain(1))
    ));
    assert_eq!(
        rig.journal(),
        [
            "fleet.settle",
            "store.save_clean_delta",
            "store.claim(cli:saved)",
            "store.load(cli:saved)",
            "store.release(cli:saved)@active=cli:departing",
        ]
    );
    assert_eq!(rig.identity(), OLD_KEY);
    assert_eq!(messages.len(), 2);
    assert_eq!(rig.roster.unwrap().records.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn a_live_row_without_a_fleet_refuses_before_anything() {
    let rig = rig_with_target(FreshOptions {
        roster: Some((2, 2)),
        ..FreshOptions::default()
    });
    let mut messages = conversation("");
    let mut runtime = rig.runtime();
    let err = rig
        .resume
        .execute("saved", &mut messages, None, &mut runtime)
        .await
        .expect_err("refused");
    assert_eq!(
        err.to_string(),
        "2 live subagent(s) but no fleet teardown is available; the current session was kept"
    );
    assert!(rig.journal().is_empty());
}

#[tokio::test]
async fn resuming_the_current_key_releases_nothing_and_still_restores() {
    let rig = build_fresh_rig(FreshOptions::default());
    rig.store.seed(Session {
        key: SessionIdentity::from_persisted_key(OLD_KEY),
        messages: vec![Message::user("on disk")],
        workflow_run: None,
        subagent_roster: Vec::new(),
    });
    let mut messages = conversation("");
    let mut runtime = rig.runtime();
    let resumed = rig
        .resume
        .execute("departing", &mut messages, None, &mut runtime)
        .await
        .expect("resumed");
    assert_eq!(resumed.identity.runtime_key(), OLD_KEY);
    assert!(
        rig.store.released.lock().unwrap().is_empty(),
        "the claim we hold is not released under our own feet"
    );
    assert_eq!(rig.store.claimed.lock().unwrap().as_slice(), [OLD_KEY]);
    assert_eq!(
        messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>(),
        ["on disk"]
    );
    assert_eq!(runtime.propagated, [OLD_KEY]);
}

/// A rig whose loop already owns `OLD_KEY`, optionally with a saved file.
fn rig_on_its_own_key(options: FreshOptions, saved: bool) -> FreshRig {
    let rig = build_fresh_rig(options);
    if saved {
        rig.store.seed(Session {
            key: SessionIdentity::from_persisted_key(OLD_KEY),
            messages: vec![Message::user("on disk")],
            workflow_run: None,
            subagent_roster: Vec::new(),
        });
    }
    rig
}

/// Resume the loop's own key and expect a refusal: whatever the failure
/// after the claim step, the claim is the running loop's own (the store's
/// claim is re-entrant, so the step took nothing new) and is never
/// released (#1995). Returns the error and the journal.
async fn refused_on_its_own_key(
    rig: &FreshRig,
    settled: bool,
) -> (ResumeSavedSessionError, Vec<String>) {
    let mut messages = conversation("");
    let mut runtime = rig.runtime();
    let fleet = rig.settled_fleet();
    let fleet =
        settled.then_some(&fleet as &dyn crate::application::sessions::ports::FleetSettlement);
    let err = rig
        .resume
        .execute("departing", &mut messages, fleet, &mut runtime)
        .await
        .expect_err("refused");
    assert_eq!(rig.identity(), OLD_KEY);
    assert_eq!(messages.len(), 2, "the live conversation is kept");
    assert!(runtime.propagated.is_empty());
    assert!(
        rig.store.released.lock().unwrap().is_empty(),
        "the loop's own claim must survive the refusal: {err}"
    );
    (err, rig.journal())
}

#[tokio::test]
async fn a_missing_file_for_the_current_key_keeps_the_loops_own_claim() {
    let rig = rig_on_its_own_key(FreshOptions::default(), false);
    let (err, journal) = refused_on_its_own_key(&rig, false).await;
    assert_eq!(err.to_string(), "session not found: departing");
    assert_eq!(
        journal,
        [
            "store.save_clean_delta",
            "store.claim(cli:departing)",
            "store.load(cli:departing)",
        ]
    );
}

#[tokio::test]
async fn a_load_error_for_the_current_key_keeps_the_loops_own_claim() {
    let rig = rig_on_its_own_key(FreshOptions::default(), true);
    rig.store.fail_load.store(true, Ordering::SeqCst);
    let (err, journal) = refused_on_its_own_key(&rig, false).await;
    let err = err.to_string();
    assert!(err.starts_with("failed to load session:"), "{err}");
    assert_eq!(journal.last().unwrap(), "store.load(cli:departing)");
}

#[tokio::test]
async fn a_scope_refusal_for_the_current_key_keeps_the_loops_own_claim() {
    let options = FreshOptions {
        legacy_homes: true,
        ..FreshOptions::default()
    };
    let rig = rig_on_its_own_key(options, true);
    let (err, journal) = refused_on_its_own_key(&rig, false).await;
    assert!(matches!(err, ResumeSavedSessionError::Scope(_)), "{err}");
    assert_eq!(journal.last().unwrap(), "store.load(cli:departing)");
}

#[tokio::test]
async fn a_kept_roster_for_the_current_key_keeps_the_loops_own_claim() {
    let options = FreshOptions {
        roster: Some((1, 3)),
        ..FreshOptions::default()
    };
    let rig = rig_on_its_own_key(options, true);
    let (err, journal) = refused_on_its_own_key(&rig, true).await;
    let err = err.to_string();
    assert!(err.contains("live"), "{err}");
    assert_eq!(journal.last().unwrap(), "store.load(cli:departing)");
    assert_eq!(rig.roster.unwrap().records.load(Ordering::SeqCst), 3);
}

/// The counterpart: a scope refusal of another key releases the claim the
/// step just took.
#[tokio::test]
async fn a_scope_refusal_of_another_key_releases_the_claim_just_taken() {
    let rig = rig_with_target(FreshOptions {
        legacy_homes: true,
        ..FreshOptions::default()
    });
    let mut messages = conversation("");
    let mut runtime = rig.runtime();
    let err = rig
        .resume
        .execute("saved", &mut messages, None, &mut runtime)
        .await
        .expect_err("refused");
    assert!(matches!(err, ResumeSavedSessionError::Scope(_)), "{err}");
    assert_eq!(
        rig.journal().last().unwrap(),
        "store.release(cli:saved)@active=cli:departing"
    );
    assert_eq!(rig.identity(), OLD_KEY);
}

#[tokio::test]
async fn every_accepted_spelling_reaches_the_store_under_its_identity() {
    for (spelled, key) in [
        ("saved", "cli:saved"),
        ("cli:saved", "cli:saved"),
        ("chat-1750000000-abc", "chat-1750000000-abc"),
    ] {
        let rig = build_fresh_rig(FreshOptions::default());
        rig.store
            .seed(Session::new(SessionIdentity::from_persisted_key(key)));
        let mut messages = conversation("");
        let mut runtime = rig.runtime();
        let resumed = rig
            .resume
            .execute(spelled, &mut messages, None, &mut runtime)
            .await
            .unwrap_or_else(|e| panic!("{spelled}: {e}"));
        assert_eq!(resumed.identity.runtime_key(), key);
        assert_eq!(resumed.name, spelled);
        assert_eq!(rig.store.claimed.lock().unwrap().as_slice(), [key]);
        assert_eq!(rig.identity(), key);
    }
}

#[tokio::test]
async fn a_killing_exit_armed_for_the_departing_session_is_dropped_by_the_switch() {
    let rig = rig_with_target(FreshOptions::default());
    rig.state.write().await.set_killing_exit(true);
    let mut messages = conversation("");
    let mut runtime = rig.runtime();
    rig.resume
        .execute("saved", &mut messages, None, &mut runtime)
        .await
        .expect("resumed");
    assert!(!rig.state.read().await.killing_exit());
}

// ─── Startup open ────────────────────────────────────────────────────────

#[tokio::test]
async fn startup_open_claims_then_loads_the_composed_identity_and_sets_the_watermark() {
    let rig = build_fresh_rig(FreshOptions::default());
    rig.store.seed(Session {
        key: SessionIdentity::from_persisted_key(OLD_KEY),
        messages: vec![Message::user("a"), Message::assistant("b", vec![])],
        workflow_run: Some(saved_run()),
        subagent_roster: Vec::new(),
    });
    let opened = rig.resume.open_at_startup().await.expect("opened");
    assert_eq!(opened.messages.len(), 2);
    assert_eq!(opened.workflow_run, Some(saved_run()));
    assert_eq!(rig.watermark(), 2);
    assert_eq!(
        rig.journal(),
        ["store.claim(cli:departing)", "store.load(cli:departing)"]
    );
}

#[tokio::test]
async fn startup_open_of_a_new_key_yields_an_empty_session() {
    let rig = build_fresh_rig(FreshOptions::default());
    let opened = rig.resume.open_at_startup().await.expect("opened");
    assert!(opened.messages.is_empty());
    assert!(opened.workflow_run.is_none());
    assert_eq!(rig.watermark(), 0);
    assert_eq!(rig.store.claimed.lock().unwrap().as_slice(), [OLD_KEY]);
}

#[tokio::test]
async fn startup_open_of_an_ephemeral_run_touches_the_store_not_at_all() {
    for (ephemeral, identity) in [(true, OLD_KEY), (false, ""), (true, "")] {
        let rig = build_fresh_rig(FreshOptions {
            ephemeral,
            current_identity: identity,
            ..FreshOptions::default()
        });
        let opened = rig.resume.open_at_startup().await.expect("opened");
        assert!(opened.messages.is_empty());
        assert!(rig.journal().is_empty(), "{ephemeral} {identity:?}");
    }
}

#[tokio::test]
async fn startup_open_refuses_a_key_owned_elsewhere_without_loading() {
    let rig = build_fresh_rig(FreshOptions::default());
    *rig.store.owned_elsewhere.lock().unwrap() = Some(OLD_KEY.into());
    let err = rig.resume.open_at_startup().await.expect_err("refused");
    assert_eq!(
        err.to_string(),
        "session error: session cli:departing is owned by another live process"
    );
    assert_eq!(rig.journal(), ["store.claim(cli:departing)"]);
}

#[tokio::test]
async fn startup_open_releases_the_claim_when_the_load_fails() {
    let rig = build_fresh_rig(FreshOptions::default());
    rig.store.fail_load.store(true, Ordering::SeqCst);
    let err = rig.resume.open_at_startup().await.expect_err("failed");
    assert_eq!(
        err.to_string(),
        "failed to load session: session error: corrupt session file"
    );
    assert_eq!(rig.store.released.lock().unwrap().as_slice(), [OLD_KEY]);
}

#[test]
fn debug_names_the_transaction_and_its_mode() {
    let rig = build_fresh_rig(FreshOptions::default());
    assert_eq!(
        format!("{:?}", rig.resume),
        "ResumeSavedSession { ephemeral: false, .. }"
    );
}
