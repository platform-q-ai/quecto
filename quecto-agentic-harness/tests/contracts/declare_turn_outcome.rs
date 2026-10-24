use quecto::application::agent_turn::{
    declaration::TurnDeclarationService, ports::DeclareTurnOutcome,
};
use quecto::domain::turn_declaration::{LaunchAuthority, TurnIdentity, TurnIntent};

#[test]
fn declare_turn_outcome_port_preserves_receipt_on_rejected_calls() {
    let id = TurnIdentity {
        session_key: "root".into(),
        outcome_epoch: "epoch".into(),
        turn_id: "turn".into(),
    };
    let service = TurnDeclarationService::new(id.clone());
    let port: &dyn DeclareTurnOutcome = &service;
    let receipt = port
        .declare(
            LaunchAuthority::TopLevelParent,
            &id,
            TurnIntent::Waiting,
            " running ",
        )
        .unwrap();
    assert_eq!(receipt.reason, "running");
    assert_eq!(receipt.identity, id);
    assert_eq!(receipt.intent, TurnIntent::Waiting);
    assert!(
        port.declare(LaunchAuthority::Child, &id, TurnIntent::Done, "complete")
            .is_err()
    );
    assert!(
        port.declare(LaunchAuthority::TopLevelParent, &id, TurnIntent::Done, "")
            .is_err()
    );
    let mut stale = id.clone();
    stale.turn_id = "old".into();
    assert!(
        port.declare(
            LaunchAuthority::TopLevelParent,
            &stale,
            TurnIntent::Done,
            "late"
        )
        .is_err()
    );
    assert_eq!(service.snapshot().unwrap(), Some(receipt));
    let done = port
        .declare(
            LaunchAuthority::TopLevelParent,
            &id,
            TurnIntent::Done,
            "complete",
        )
        .unwrap();
    assert_eq!(service.snapshot().unwrap(), Some(done));
}
