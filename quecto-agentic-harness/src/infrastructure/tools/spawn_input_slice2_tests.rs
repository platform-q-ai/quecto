//! Slice 2 (#1369): `{"mode":"existing","ref"|"name"}` spawn input.

use super::*;
use serde_json::json;

#[test]
fn existing_mode_with_ref_parses_to_a_ref_target() {
    let parsed = parse_container_selection(&json!({"container":{"mode":"existing","ref":"C1"}}));
    assert_eq!(
        parsed,
        Ok(ContainerSelection::Existing {
            target: crate::domain::environment_registry::EnvironmentTarget::Ref("C1".into())
        }),
        "existing mode with a ref must carry the ref target"
    );
}

#[test]
fn existing_mode_with_name_parses_to_a_name_target() {
    let parsed =
        parse_container_selection(&json!({"container":{"mode":"existing","name":"review-env"}}));
    assert_eq!(
        parsed,
        Ok(ContainerSelection::Existing {
            target: crate::domain::environment_registry::EnvironmentTarget::Name(
                "review-env".into()
            )
        }),
        "existing mode with a name must carry the name target"
    );
}

#[test]
fn existing_mode_requires_exactly_one_target() {
    let err = parse_container_selection(
        &json!({"container":{"mode":"existing","ref":"C1","name":"review-env"}}),
    )
    .unwrap_err();
    assert!(
        err.contains("exactly one"),
        "ref+name must be rejected as over-specified: {err}"
    );
    let err = parse_container_selection(&json!({"container":{"mode":"existing"}})).unwrap_err();
    assert!(
        err.contains("exactly one"),
        "existing without ref or name must demand exactly one target: {err}"
    );
}

#[test]
fn existing_mode_rejects_new_only_fields() {
    let err = parse_container_selection(
        &json!({"container":{"mode":"existing","ref":"C1","container_config":"alternate"}}),
    )
    .unwrap_err();
    assert!(
        err.contains("container_config"),
        "container_config is only valid for mode 'new': {err}"
    );
    let err = parse_container_selection(
        &json!({"container":{"mode":"existing","ref":"C1","repo":"https://example.invalid/r.git"}}),
    )
    .unwrap_err();
    // repo no longer exists anywhere on the surface: plain unknown field.
    assert!(err.contains("unknown container field 'repo'"), "{err}");
}

#[test]
fn existing_mode_target_must_be_a_string() {
    let err =
        parse_container_selection(&json!({"container":{"mode":"existing","ref":1}})).unwrap_err();
    assert!(
        err.contains("ref") && err.contains("must be a string"),
        "non-string ref must be rejected with a type error: {err}"
    );
}

#[test]
fn new_mode_accepts_optional_environment_name() {
    let parsed =
        parse_container_selection(&json!({"container":{"mode":"new","name":"review-env"}}));
    assert_eq!(
        parsed,
        Ok(ContainerSelection::New {
            container_config: None,
            name: Some("review-env".into()),
        }),
        "mode new must carry the optional environment name"
    );
}
