//! #2708: the bound declaration port the Native tool holds captures only
//! the admitted turn, and declares only for it.
use quecto::application::agent_turn::ports::BoundTurnDeclaration;
use quecto::application::agent_turn::use_cases::parent_outcome::{
    ParentOutcomeRuntime, TurnAdmission, TurnEnd,
};
use quecto::domain::agents::value_objects::turn_outcome::WorkLiveness;
use quecto::domain::turn_declaration::{LaunchAuthority, TurnIntent};

#[test]
fn bound_turn_declaration_port_captures_only_the_admitted_turn() {
    let runtime = ParentOutcomeRuntime::new(|| "epoch".into());
    let port: &dyn BoundTurnDeclaration = &runtime;
    assert_eq!(port.capture(), None, "nothing admitted, nothing captured");
    let turn = runtime
        .admit(
            "cli:root",
            TurnAdmission::External {
                episode_id: "episode".into(),
            },
            "opener",
        )
        .expect("an external instruction is admitted");
    let identity = port.capture().expect("the admitted turn");
    assert_eq!(
        (
            identity.session_key.as_str(),
            identity.outcome_epoch.as_str(),
            identity.turn_id.as_str()
        ),
        ("cli:root", "epoch", "opener")
    );
    let receipt = port
        .declare(
            LaunchAuthority::TopLevelParent,
            &identity,
            TurnIntent::Done,
            " complete ",
        )
        .unwrap();
    assert_eq!(receipt.reason, "complete");
    assert!(
        port.declare(LaunchAuthority::Child, &identity, TurnIntent::Done, "x")
            .is_err()
    );
    let _ = runtime.complete(turn, TurnEnd::Completed(WorkLiveness::Empty));
    assert_eq!(port.capture(), None, "a completed turn is released");
    assert!(
        port.declare(
            LaunchAuthority::TopLevelParent,
            &identity,
            TurnIntent::Done,
            "late"
        )
        .is_err(),
        "a released turn cannot declare"
    );
}
