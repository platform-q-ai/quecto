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
    // repo was removed from the surface entirely (#1410): configs own it.
    assert!(parse_container_selection(&json!({"container":{"mode":"new","repo":"r"}})).is_err());
    assert!(parse_container_selection(&json!({"container":{"mode":"existing"}})).is_err());
    assert!(parse_container_selection(&json!({"container":{"mode":"other"}})).is_err());
    assert!(
        parse_container_selection(&json!({"container":{"mode":"new","container_config":1}}))
            .is_err()
    );
    assert!(parse_container_selection(&json!({"container":{"mode":"new","name":1}})).is_err());
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
    let parsed =
        parse_container_selection(&json!({"container":{"mode":"new","container_config":"s"}}))
            .unwrap();
    assert_eq!(
        parsed,
        ContainerSelection::New {
            container_config: Some("s".into()),
            name: None,
        }
    );
}

fn new_container() -> ContainerSelection {
    ContainerSelection::New {
        container_config: None,
        name: None,
    }
}

/// #2461 review: a coordinator is an explicit opt-in; absent or false is an
/// ordinary child wherever it runs.
#[test]
fn coordinator_defaults_to_an_ordinary_child() {
    for args in [
        json!({}),
        json!({"coordinator": null}),
        json!({"coordinator": false}),
    ] {
        assert_eq!(parse_coordinator(&args, &new_container(), false), Ok(false));
    }
}

/// #2461 review: a coordinator starts its swarm in a fresh container, so it
/// is accepted only for a new container; quoted booleans read as booleans.
#[test]
fn coordinator_is_accepted_only_for_a_new_container() {
    for args in [json!({"coordinator": true}), json!({"coordinator": "true"})] {
        assert_eq!(parse_coordinator(&args, &new_container(), false), Ok(true));
    }
    let existing = ContainerSelection::Existing {
        target:
            crate::domain::environments::entities::environment_registry::EnvironmentTarget::Name(
                "c".into(),
            ),
    };
    for container in [ContainerSelection::Local, existing] {
        let err = parse_coordinator(&json!({"coordinator": true}), &container, false).unwrap_err();
        assert!(err.contains("new container"), "{err}");
    }
}

/// #2461 review: a workflow agent cannot create a swarm, so it cannot be one's
/// coordinator; a value that is not a boolean is refused, not ignored.
#[test]
fn coordinator_refuses_workflow_and_non_boolean_values() {
    for workflow in [
        json!({"coordinator": true, "workflow": true}),
        json!({"coordinator": true, "workflow_spec": {"template": {}}}),
    ] {
        let err = parse_coordinator(&workflow, &new_container(), false).unwrap_err();
        assert!(err.contains("workflow"), "{err}");
    }
    for value in [json!(1), json!("yes"), json!({})] {
        let err =
            parse_coordinator(&json!({"coordinator": value}), &new_container(), false).unwrap_err();
        assert!(err.contains("coordinator must be a boolean"), "{err}");
    }
}

/// #2461 review: a swarm member never launches a coordinator; the refusal
/// comes before anything is written for the launch.
#[test]
fn a_swarm_member_cannot_launch_a_coordinator() {
    let err = parse_coordinator(&json!({"coordinator": true}), &new_container(), true).unwrap_err();
    assert!(err.contains("swarm member"), "{err}");
    assert_eq!(
        parse_coordinator(&json!({}), &new_container(), true),
        Ok(false)
    );
}
