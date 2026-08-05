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
            container_script: Some("s".into())
        }
    );
}

#[test]
fn validate_script_rejects_missing_or_unsafe_argv() {
    assert!(
        validate_script(&ContainerScriptConfig {
            create: vec![],
            cleanup: vec![]
        })
        .is_err()
    );
    assert!(
        validate_script(&ContainerScriptConfig {
            create: vec!["".into()],
            cleanup: vec![]
        })
        .is_err()
    );
    assert!(
        validate_script(&ContainerScriptConfig {
            create: vec!["ok".into()],
            cleanup: vec!["bad\0".into()]
        })
        .is_err()
    );
}

#[test]
fn cleanup_command_is_once_consumable_by_prepared_child() {
    let cmd = cleanup_command(Some("C-test"), &["echo".into()]);
    assert!(cmd.is_some());
    assert!(cleanup_command(None, &["echo".into()]).is_none());
    assert!(cleanup_command(Some("C-test"), &[]).is_none());
}
