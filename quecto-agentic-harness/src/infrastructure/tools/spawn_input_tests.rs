use super::*;
use serde_json::json;

#[test]
fn parse_rejects_runtime_specific_fields() {
    for field in ["branch", "pr", "image", "runtime"] {
        let err =
            parse_container_selection(&json!({"container":{"mode":"new", field:"x"}})).unwrap_err();
        assert!(err.contains(field), "{err}");
    }
}

#[test]
fn parse_rejects_invalid_container_shapes_and_modes() {
    assert!(parse_container_selection(&json!({"container":"new"})).is_err());
    assert!(parse_container_selection(&json!({"container":{"repo":"r"}})).is_err());
    assert!(parse_container_selection(&json!({"container":{"mode":"existing"}})).is_err());
    assert!(parse_container_selection(&json!({"container":{"mode":"other"}})).is_err());
    assert!(parse_container_selection(&json!({"container":{"mode":"new","repo":1}})).is_err());
    assert!(
        parse_container_selection(&json!({"container":{"mode":"new","container_script":1}}))
            .is_err()
    );
}

#[test]
fn parse_accepts_true_false_and_new_object() {
    assert!(matches!(
        parse_container_selection(&json!({})).unwrap(),
        ContainerSelection::Local
    ));
    assert!(matches!(
        parse_container_selection(&json!({"container":false})).unwrap(),
        ContainerSelection::Local
    ));
    assert!(matches!(
        parse_container_selection(&json!({"container":true})).unwrap(),
        ContainerSelection::New { .. }
    ));
    let parsed = parse_container_selection(
        &json!({"container":{"mode":"new","repo":"r","container_script":"s"}}),
    )
    .unwrap();
    assert_eq!(
        parsed,
        ContainerSelection::New {
            repo: Some("r".into()),
            container_script: Some("s".into()),
            name: None,
        }
    );
}
