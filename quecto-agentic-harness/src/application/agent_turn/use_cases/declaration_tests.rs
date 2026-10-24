use super::*;
use std::sync::Arc;

fn identity() -> TurnIdentity {
    TurnIdentity {
        session_key: "root".into(),
        outcome_epoch: "epoch-1".into(),
        turn_id: "turn-1".into(),
    }
}

fn accept(
    service: &TurnDeclarationService,
    id: &TurnIdentity,
    intent: TurnIntent,
    reason: &str,
) -> TurnDeclaration {
    service
        .declare(LaunchAuthority::TopLevelParent, id, intent, reason)
        .expect("missing acceptance: trusted active top-level declaration must succeed")
}

macro_rules! valid_intent {
    ($name:ident, $intent:ident) => {
        #[test]
        fn $name() {
            let id = identity();
            let service = TurnDeclarationService::new(id.clone());
            let receipt = accept(&service, &id, TurnIntent::$intent, "  ready  ");
            assert_eq!(
                receipt,
                TurnDeclaration {
                    identity: id,
                    intent: TurnIntent::$intent,
                    reason: "ready".into()
                }
            );
            assert_eq!(service.snapshot().unwrap(), Some(receipt));
        }
    };
}
valid_intent!(waiting_is_typed_and_trimmed, Waiting);
valid_intent!(blocked_is_typed_and_trimmed, Blocked);
valid_intent!(asking_is_typed_and_trimmed, Asking);
valid_intent!(done_is_typed_and_trimmed, Done);

#[test]
fn fresh_and_ephemeral_roots_have_no_declaration() {
    let id = identity();
    let service = TurnDeclarationService::new(id.clone());
    assert_eq!(service.snapshot().unwrap(), None);
    accept(&service, &id, TurnIntent::Done, "complete");
    let fresh = TurnDeclarationService::new(id);
    assert_eq!(
        fresh.snapshot().unwrap(),
        None,
        "new root must not recover declaration from prior instance"
    );
    assert!(
        service.snapshot().unwrap().is_some(),
        "instances must be isolated"
    );
}

#[test]
fn empty_constructor_is_a_baseline_characterization() {
    assert_eq!(
        TurnDeclarationService::new(identity()).snapshot().unwrap(),
        None
    );
}

#[test]
fn latest_valid_waiting_then_done_wins() {
    let id = identity();
    let service = TurnDeclarationService::new(id.clone());
    accept(&service, &id, TurnIntent::Waiting, "child running");
    let done = accept(&service, &id, TurnIntent::Done, "complete");
    assert_eq!(service.snapshot().unwrap(), Some(done));
}

macro_rules! invalid_reason {
    ($name:ident, $reason:expr) => {
        #[test]
        fn $name() {
            let id = identity();
            let service = TurnDeclarationService::new(id.clone());
            let error = service.declare(LaunchAuthority::TopLevelParent, &id, TurnIntent::Done, $reason).unwrap_err();
            assert_eq!(error.to_string(), "tool error: Choose waiting, blocked, asking, or done and provide a nonempty one-line reason of at most 512 characters.", "missing reason validation");
            assert_eq!(service.snapshot().unwrap(), None);
            let prior = accept(&service, &id, TurnIntent::Waiting, "running");
            assert!(service.declare(LaunchAuthority::TopLevelParent, &id, TurnIntent::Done, $reason).is_err());
            assert_eq!(service.snapshot().unwrap(), Some(prior), "invalid reason must preserve valid declaration");
        }
    };
}
invalid_reason!(empty_reason_rejected, "");
invalid_reason!(whitespace_reason_rejected, " \t\u{2003} ");
invalid_reason!(embedded_cr_rejected, "one\rtwo");
invalid_reason!(embedded_lf_rejected, "one\ntwo");
invalid_reason!(crlf_rejected, "one\r\ntwo");
invalid_reason!(ascii_513_rejected, &"x".repeat(513));
invalid_reason!(multibyte_513_rejected, &"界".repeat(513));

macro_rules! boundary_reason {
    ($name:ident, $reason:expr) => {
        #[test]
        fn $name() {
            let id = identity();
            let service = TurnDeclarationService::new(id.clone());
            let reason = $reason;
            let receipt = accept(&service, &id, TurnIntent::Done, &reason);
            assert_eq!(receipt.reason, reason.trim());
        }
    };
}
boundary_reason!(ascii_512_accepted, "x".repeat(512));
boundary_reason!(unicode_scalars_not_bytes_512_accepted, "界".repeat(512));
boundary_reason!(
    surrounding_unicode_whitespace_trimmed,
    "\u{2003}ready\t ".to_string()
);

macro_rules! unauthorized {
    ($name:ident, $role:ident) => {
        #[test]
        fn $name() {
            let id = identity();
            let service = TurnDeclarationService::new(id.clone());
            let error = service.declare(LaunchAuthority::$role, &id, TurnIntent::Done, "complete").unwrap_err();
            assert_eq!(error.to_string(), "tool error: Turn outcomes may be declared only by the active top-level parent turn.", "missing trusted role enforcement");
            assert_eq!(service.snapshot().unwrap(), None);
            let prior = accept(&service, &id, TurnIntent::Waiting, "running");
            assert!(service.declare(LaunchAuthority::$role, &id, TurnIntent::Done, "complete").is_err());
            assert_eq!(service.snapshot().unwrap(), Some(prior));
        }
    };
}
unauthorized!(child_rejected_without_mutation, Child);
unauthorized!(
    child_coordinator_rejected_without_mutation,
    ChildCoordinator
);

macro_rules! stale {
    ($name:ident, $field:ident) => {
        #[test]
        fn $name() {
            let id = identity();
            let service = TurnDeclarationService::new(id.clone());
            let mut old = id.clone();
            old.$field = "stale".into();
            let error = service
                .declare(
                    LaunchAuthority::TopLevelParent,
                    &old,
                    TurnIntent::Done,
                    "complete",
                )
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                "tool error: This declaration does not belong to the active turn.",
                "missing identity rejection"
            );
            assert_eq!(service.snapshot().unwrap(), None);
            let prior = accept(&service, &id, TurnIntent::Waiting, "running");
            assert!(
                service
                    .declare(
                        LaunchAuthority::TopLevelParent,
                        &old,
                        TurnIntent::Done,
                        "late"
                    )
                    .is_err()
            );
            assert_eq!(service.snapshot().unwrap(), Some(prior));
        }
    };
}
stale!(stale_session_preserves_state, session_key);
stale!(stale_epoch_preserves_state, outcome_epoch);
stale!(stale_turn_preserves_state, turn_id);

#[test]
fn outer_run_resets_but_retry_and_tool_iteration_retain() {
    let id = identity();
    let service = TurnDeclarationService::new(id.clone());
    let prior = accept(&service, &id, TurnIntent::Waiting, "running");
    for boundary in [TurnBoundary::ProviderRetry, TurnBoundary::ToolIteration] {
        service
            .enter(boundary, &id)
            .expect("internal attempt must retain declaration");
        assert_eq!(service.snapshot().unwrap(), Some(prior.clone()));
    }
    let mut next = id.clone();
    next.turn_id = "turn-2".into();
    service
        .enter(TurnBoundary::OuterRun, &next)
        .expect("new admitted outer run must reset declaration");
    assert_eq!(service.snapshot().unwrap(), None);
    assert!(
        service
            .declare(
                LaunchAuthority::TopLevelParent,
                &id,
                TurnIntent::Done,
                "late"
            )
            .is_err()
    );
    assert_eq!(service.snapshot().unwrap(), None);
    let current = accept(&service, &next, TurnIntent::Done, "new complete");
    assert_eq!(service.snapshot().unwrap(), Some(current));
}

#[test]
fn stale_lifecycle_cannot_clear_current_declaration() {
    let id = identity();
    let service = TurnDeclarationService::new(id.clone());
    let prior = accept(&service, &id, TurnIntent::Done, "complete");
    let mut stale = id;
    stale.outcome_epoch = "old".into();
    for boundary in [TurnBoundary::ProviderRetry, TurnBoundary::ToolIteration] {
        assert!(service.enter(boundary, &stale).is_err());
        assert_eq!(service.snapshot().unwrap(), Some(prior.clone()));
    }
}

#[test]
fn concurrent_current_and_stale_writes_preserve_current_receipt() {
    let id = identity();
    let service = Arc::new(TurnDeclarationService::new(id.clone()));
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let handles: Vec<_> = [false, true]
        .into_iter()
        .map(|stale| {
            let service = service.clone();
            let barrier = barrier.clone();
            let mut identity = id.clone();
            if stale {
                identity.turn_id = "old".into();
            }
            std::thread::spawn(move || {
                barrier.wait();
                (
                    stale,
                    service.declare(
                        LaunchAuthority::TopLevelParent,
                        &identity,
                        TurnIntent::Done,
                        "concurrent",
                    ),
                )
            })
        })
        .collect();
    barrier.wait();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    let valid = results.iter().find(|e| matches!(e, (false, _))).unwrap();
    assert!(
        valid.1.is_ok(),
        "missing acceptance: concurrent current write must succeed: {:?}",
        valid.1
    );
    let old = results.iter().find(|(stale, _)| *stale).unwrap();
    assert!(old.1.is_err());
    assert_eq!(service.snapshot().unwrap(), valid.1.as_ref().ok().cloned());
}

#[test]
fn concurrent_valid_writes_are_atomic_then_sequential_final_write_wins() {
    let id = identity();
    let service = Arc::new(TurnDeclarationService::new(id.clone()));
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let handles: Vec<_> = [TurnIntent::Waiting, TurnIntent::Done]
        .into_iter()
        .map(|intent| {
            let service = service.clone();
            let barrier = barrier.clone();
            let id = id.clone();
            std::thread::spawn(move || {
                barrier.wait();
                accept(&service, &id, intent, "atomic")
            })
        })
        .collect();
    barrier.wait();
    let receipts: Vec<_> = handles
        .into_iter()
        .map(|h| h.join().expect("concurrent accepted write must not panic"))
        .collect();
    let snapshot = service
        .snapshot()
        .unwrap()
        .expect("one whole receipt must survive");
    assert!(receipts.contains(&snapshot));
    let final_receipt = accept(&service, &id, TurnIntent::Done, "last completed");
    assert_eq!(service.snapshot().unwrap(), Some(final_receipt));
}

#[test]
fn admitted_outer_run_is_a_callable_lifecycle_boundary() {
    let id = identity();
    let service = TurnDeclarationService::new(id.clone());
    let mut next = id;
    next.turn_id = "turn-2".into();
    service
        .enter(TurnBoundary::OuterRun, &next)
        .expect("missing lifecycle acceptance: admitting a new outer run must succeed");
    assert_eq!(service.snapshot().unwrap(), None);
    let receipt = accept(&service, &next, TurnIntent::Done, "complete");
    assert_eq!(service.snapshot().unwrap(), Some(receipt));
}

#[test]
fn internal_boundaries_do_not_require_a_prior_declaration() {
    let id = identity();
    let service = TurnDeclarationService::new(id.clone());
    for boundary in [TurnBoundary::ProviderRetry, TurnBoundary::ToolIteration] {
        service
            .enter(boundary, &id)
            .expect("missing lifecycle acceptance: internal attempts must retain even empty state");
        assert_eq!(service.snapshot().unwrap(), None);
    }
}

boundary_reason!(
    trimmed_ascii_bound_is_512_scalars,
    format!("  {}\t", "x".repeat(512))
);
boundary_reason!(supplementary_unicode_512_scalars, "🦀".repeat(512));
