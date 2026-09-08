//! Wire round-trips: every operation and reply body survives JSON encoding,
//! and typed conversions map both directions.
use super::*;
use std::collections::BTreeMap;

fn proposal_wire() -> ProposalWire {
    ProposalWire {
        groups: BTreeMap::from([(
            "g".into(),
            GroupPolicyWire {
                capacity: 2,
                reserve: 1,
                min_interval_ms: 5,
                queue_capacity: 8,
                queue_timeout_ms: 100,
                attempt_timeout_ms: 200,
                fallback_base_ms: 50,
                max_cooldown_ms: 500,
            },
        )]),
        aliases: BTreeMap::from([("acct".into(), "g".into())]),
        max_scopes: 4,
        terminal_capacity: 8,
        bindings: BTreeMap::from([("openai".into(), "acct".into())]),
    }
}

#[test]
fn requests_and_replies_round_trip_through_json() {
    let credential = CredentialWire {
        scope: ScopeWire {
            epoch: 2,
            serial: 7,
        },
        token: "t".into(),
    };
    let ops = vec![
        Op::Hello {
            version: 1,
            capability: CAPABILITY_DIRECT.into(),
        },
        Op::RegisterRoot {
            class: ClassWire::Interactive,
            owner_token: "o".into(),
        },
        Op::Bind {
            credential: credential.clone(),
        },
        Op::RegisterChild,
        Op::Acquire {
            sequence: 1,
            alias: "acct".into(),
        },
        Op::Cancel { sequence: 1 },
        Op::Feedback {
            sequence: 1,
            report: 1,
            feedback: ThrottleWire::Until { deadline_ms: 9 },
        },
        Op::Complete {
            sequence: 1,
            feedback: FeedbackWire::Throttle { delay_ms: 3 },
        },
        Op::Status { sequence: 1 },
        Op::Retire,
        Op::RetireChild {
            scope: ScopeWire {
                epoch: 2,
                serial: 8,
            },
        },
        Op::Inspect,
        Op::Reset,
    ];
    for (id, op) in ops.into_iter().enumerate() {
        let request = Request {
            id: id as u64,
            op: op.clone(),
        };
        let bytes = serde_json::to_vec(&request).unwrap();
        let decoded: Request = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded, request, "{op:?}");
        assert!(!format!("{decoded:?}").is_empty());
    }
    let bodies = vec![
        Body::Hello {
            version: 1,
            role: Role::Client,
            epoch: 3,
            proposal: proposal_wire(),
        },
        Body::Credential { credential },
        Body::Bound { next_sequence: 4 },
        Body::Ok,
        Body::State {
            state: StateWire::Active {
                started_ms: 1,
                deadline_ms: 2,
                cancellation_required: false,
            },
        },
        Body::Granted {
            deadline_ms: 5,
            receipt_ms: 1,
            receipt_wall_ms: 2,
        },
        Body::Status {
            epoch: 3,
            journal_healthy: true,
            live_scopes: 1,
            groups: BTreeMap::from([(
                "g".into(),
                SnapshotWire {
                    active: 1,
                    queued: 0,
                    uncertain: 0,
                    cooldown_until_ms: 0,
                    unavailable: false,
                },
            )]),
        },
        Body::Reset { epoch: 4 },
        Body::Error {
            code: ErrorCode::Timeout,
            reason: "late".into(),
        },
        Body::CancelRequired { sequence: 1 },
        Body::Revoked,
    ];
    for body in bodies {
        let reply = Reply {
            id: Some(1),
            body: body.clone(),
        };
        let bytes = serde_json::to_vec(&reply).unwrap();
        let decoded: Reply = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded, reply, "{body:?}");
    }
}

#[test]
fn typed_conversions_are_lossless() {
    for state in [
        RequestState::Queued { deadline: 1 },
        RequestState::Active {
            started: 1,
            deadline: 2,
            cancellation_required: true,
        },
        RequestState::Terminal(TerminalOutcome::Finished),
        RequestState::Terminal(TerminalOutcome::Cancelled),
        RequestState::Terminal(TerminalOutcome::TimedOut),
    ] {
        assert_eq!(RequestState::from(StateWire::from(state)), state);
    }
    for feedback in [
        ThrottleFeedback::NoHint { jitter: 3 },
        ThrottleFeedback::Until(9),
        ThrottleFeedback::Unavailable,
    ] {
        assert_eq!(
            ThrottleFeedback::from(ThrottleWire::from(feedback)),
            feedback
        );
    }
    for feedback in [
        Feedback::Success,
        Feedback::Failure,
        Feedback::Throttle { delay_ms: 2 },
    ] {
        assert_eq!(Feedback::from(FeedbackWire::from(feedback)), feedback);
    }
    for class in [WorkloadClass::Interactive, WorkloadClass::Background] {
        assert_eq!(WorkloadClass::from(ClassWire::from(class)), class);
    }
    let scope = ScopeId {
        epoch: 1,
        serial: 2,
    };
    assert_eq!(ScopeId::from(ScopeWire::from(scope)), scope);
    let proposal = AdmissionRuntimeProposal::try_from(proposal_wire()).unwrap();
    assert_eq!(ProposalWire::from(&proposal), proposal_wire());
    let mut invalid = proposal_wire();
    invalid.groups.get_mut("g").unwrap().reserve = 5;
    assert_eq!(
        AdmissionRuntimeProposal::try_from(invalid).err(),
        Some(AdmissionError::InvalidConfig)
    );
    let status = status_from_body(
        3,
        true,
        2,
        BTreeMap::from([(
            "g".into(),
            SnapshotWire {
                active: 1,
                queued: 2,
                uncertain: 0,
                cooldown_until_ms: 7,
                unavailable: false,
            },
        )]),
    )
    .unwrap();
    assert_eq!(status.live_scopes, 2);
    if let Body::Status { groups, .. } = status_body(&status) {
        assert_eq!(groups["g"].queued, 2);
    } else {
        panic!("status body");
    }
    assert!(
        status_from_body(
            1,
            true,
            0,
            BTreeMap::from([(
                "".into(),
                SnapshotWire {
                    active: 0,
                    queued: 0,
                    uncertain: 0,
                    cooldown_until_ms: 0,
                    unavailable: false
                }
            )])
        )
        .is_err()
    );
    for error in [
        AuthorityError::Unauthorized,
        AuthorityError::JournalUnavailable,
        AuthorityError::Admission(AdmissionError::QueueFull),
    ] {
        assert!(matches!(error_body(&error), Body::Error { .. }));
    }
}
