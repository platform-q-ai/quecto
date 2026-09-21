//! Resuming the key the loop already stands for (#1995): it reloads from
//! disk in place, and every refusal on that path keeps the loop's own claim.
use super::*;

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
        .execute(
            &ResumeRequest::restore("departing"),
            &mut messages,
            fleet,
            &mut runtime,
        )
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
            "store.claim(cli:departing)",
            "store.save_clean_delta",
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
    assert!(matches!(err, ResumeSavedSessionError::Decision(_)), "{err}");
    assert_eq!(journal, Vec::<String>::new(), "decided by the pre-flight");
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
