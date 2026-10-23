use super::*;
use crate::infrastructure::processes::parent_control::{mint_credential, presentation_json};

#[test]
fn presentation_wire_claims_only_its_own_type_and_fails_closed() {
    let credential = mint_credential();
    let line = presentation_json(&credential);
    let claimed = BindParentControlWire::claim(&format!("{line}\n"))
        .unwrap()
        .expect("claimed");
    let (generation, capability) = claimed.presented().unwrap();
    assert_eq!(generation, credential.generation);
    assert_eq!(capability, credential.capability);
    let encoded: serde_json::Value = serde_json::from_str(&line).unwrap();
    let schema: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&claimed).unwrap()).unwrap();
    assert_eq!(encoded, schema, "the launcher encodes exactly this schema");
    // Not ours: other commands, non-objects, garbage, and anything the
    // prefilter rules out without parsing.
    for other in [
        r#"{"type":"prompt","message":"hi"}"#,
        "[1,2]",
        "",
        "not json",
        r#"{"kind":"bind_parent_control"}"#,
        r#"{"type":"prompt","message":"bind_parent_control"}"#,
    ] {
        assert_eq!(
            BindParentControlWire::claim(other).unwrap(),
            None,
            "{other}"
        );
    }
    assert!(!BindParentControlWire::may_be_presentation(
        &"x".repeat(1 << 20)
    ));
    // Ours but malformed: fail closed, never dispatched as an ordinary line.
    for bad in [
        r#"{"type":"bind_parent_control"}"#,
        r#"{"type":"bind_parent_control","generation":"1","capability":"aa"}"#,
        r#"{"type":"bind_parent_control","generation":1,"capability":"aa","extra":1}"#,
    ] {
        assert!(BindParentControlWire::claim(bad).is_err(), "{bad}");
    }
    let short = BindParentControlWire {
        kind: BIND_PARENT_CONTROL.into(),
        generation: 1,
        capability: "aa".into(),
    };
    assert!(short.presented().is_err());
    assert_eq!(
        bound_ack_line(),
        "{\"type\":\"response\",\"command\":\"bind_parent_control\",\"success\":true}\n"
    );
}
