use super::*;

/// Each gate's wire name parses back to it, and only an allowlisted name
/// parses: another case, padding or an empty name names no gate.
#[test]
fn only_an_allowlisted_name_parses() {
    for gate in AdmissionGate::ALL {
        assert_eq!(AdmissionGate::parse(gate.as_str()), Some(gate));
    }
    for name in ["", "Model", "TOOL", " tool", "retry ", "model_gate", "read"] {
        assert_eq!(AdmissionGate::parse(name), None, "{name:?}");
    }
}

/// The three gates are three decisions, each a decision kind and none the
/// budget's own (`paused`, `warned`) or a plain `read`.
#[test]
fn each_gate_is_its_own_decision_kind() {
    let decisions: Vec<&str> = AdmissionGate::ALL.map(AdmissionGate::decision).to_vec();
    assert_eq!(decisions, ["model_gate", "retry_gate", "tool_gate"]);
    for decision in decisions {
        assert!(
            super::super::telemetry::decision_kind(decision),
            "{decision}"
        );
    }
}

/// A request's first send is the model gate, every later send the retry
/// gate.
#[test]
fn a_send_is_admitted_at_its_attempts_gate() {
    assert_eq!(
        AdmissionGate::for_attempt(RequestAttempt::First),
        AdmissionGate::Model
    );
    assert_eq!(
        AdmissionGate::for_attempt(RequestAttempt::Reattempt),
        AdmissionGate::Retry
    );
}
