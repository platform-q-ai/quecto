use super::*;
use crate::domain::tool::ToolDefinition;

fn def(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string().into(),
        description: "desc".into(),
        parameters_schema: r#"{"type":"object"}"#.into(),
    }
}

#[test]
fn tool_source_as_str_covers_all_variants() {
    assert_eq!(ToolSource::BundledNative.as_str(), "bundled-native");
    assert_eq!(ToolSource::Uds.as_str(), "uds");
    assert_eq!(ToolSource::Runtime.as_str(), "runtime");
}

#[test]
fn tool_availability_as_str_and_is_enabled() {
    assert_eq!(ToolAvailability::Enabled.as_str(), "enabled");
    assert_eq!(ToolAvailability::Disabled.as_str(), "disabled");
    assert!(ToolAvailability::Enabled.is_enabled());
    assert!(!ToolAvailability::Disabled.is_enabled());
}

#[test]
fn tool_descriptor_constructors_and_name() {
    let enabled =
        ToolDescriptor::enabled(def("bash"), ToolSource::BundledNative, "quecto:official");
    assert_eq!(enabled.name(), "bash");
    assert!(enabled.availability.is_enabled());
    assert_eq!(enabled.source, ToolSource::BundledNative);
    assert_eq!(enabled.owner.as_ref(), "quecto:official");

    let disabled = ToolDescriptor::new(
        def("weather"),
        ToolSource::Uds,
        "uds:client-1",
        ToolAvailability::Disabled,
    );
    assert_eq!(disabled.name(), "weather");
    assert!(!disabled.availability.is_enabled());
    assert_eq!(disabled.source, ToolSource::Uds);

    let runtime = ToolDescriptor::enabled(def("custom"), ToolSource::Runtime, "runtime");
    assert_eq!(runtime.source.as_str(), "runtime");
}

#[test]
fn rich_catalogue_state_types_have_stable_strings() {
    assert_eq!(ToolLifecycleKind::Bundled.as_str(), "bundled");
    assert_eq!(
        ToolLifecycleKind::RuntimeLoadable.as_str(),
        "runtime-loadable"
    );
    assert_eq!(
        ToolRestrictionReason::EntrypointDefault.as_str(),
        "entrypoint-default"
    );
    assert_eq!(ToolRestrictionReason::Session.as_str(), "session");
    assert_eq!(ToolRestrictionReason::Spawn.as_str(), "spawn");
    assert_eq!(
        ToolRestrictionReason::ExplicitDisable.as_str(),
        "explicit-disable"
    );
    assert_eq!(ToolHealth::Ok.as_str(), "ok");
    assert_eq!(ToolHealth::Disabled.as_str(), "disabled");
    assert_eq!(ToolHealth::Unavailable.as_str(), "unavailable");
    assert_eq!(ToolHealth::Unknown.as_str(), "unknown");
}

#[test]
fn profile_scope_helpers_and_serde_names_are_stable() {
    let cases = [
        (
            ProfileAvailabilityScope::None,
            false,
            false,
            ProfileAvailabilityScope::Parent,
            "none",
        ),
        (
            ProfileAvailabilityScope::Parent,
            true,
            false,
            ProfileAvailabilityScope::Child,
            "parent",
        ),
        (
            ProfileAvailabilityScope::Child,
            false,
            true,
            ProfileAvailabilityScope::Both,
            "child",
        ),
        (
            ProfileAvailabilityScope::Both,
            true,
            true,
            ProfileAvailabilityScope::None,
            "both",
        ),
    ];

    for (scope, parent, child, next, json) in cases {
        assert_eq!(scope.allows_parent(), parent);
        assert_eq!(scope.allows_child(), child);
        assert_eq!(scope.cycle_next(), next);
        assert_eq!(
            serde_json::to_string(&scope).unwrap(),
            format!("\"{}\"", json)
        );
        assert_eq!(
            ProfileAvailabilityScope::from_parent_child(parent, child),
            scope
        );
    }
}

#[test]
fn profile_scope_intersection_covers_all_scope_combinations() {
    use ProfileAvailabilityScope::{Both, Child, None, Parent};

    let cases = [
        (None, None, None),
        (None, Parent, None),
        (None, Child, None),
        (None, Both, None),
        (Parent, None, None),
        (Parent, Parent, Parent),
        (Parent, Child, None),
        (Parent, Both, Parent),
        (Child, None, None),
        (Child, Parent, None),
        (Child, Child, Child),
        (Child, Both, Child),
        (Both, None, None),
        (Both, Parent, Parent),
        (Both, Child, Child),
        (Both, Both, Both),
    ];

    for (left, right, expected) in cases {
        assert_eq!(left.intersection(right), expected, "{left:?} ∩ {right:?}");
    }
}
