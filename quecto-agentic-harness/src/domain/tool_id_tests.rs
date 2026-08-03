use super::*;
use crate::domain::tool_descriptor::ToolSource;

#[test]
fn stable_tool_ids_are_provider_qualified() {
    assert_eq!(
        stable_tool_id(ToolSource::BundledNative, "quecto:official-tools", "bash"),
        "tool.v1:bundled-native:21:quecto:official-tools:bash"
    );
    assert_eq!(legacy_name_tool_id("bash"), "tool.name.v0:bash");
}

#[test]
fn resolver_accepts_canonical_legacy_and_alias_ids() {
    let identity = ToolIdentity::new(
        ToolSource::BundledNative,
        "quecto:official-tools",
        "read",
        vec!["view".into()],
    );
    let mut resolver = ToolIdResolver::default();
    resolver.register(&identity).unwrap();

    for input in [
        "tool.v1:bundled-native:21:quecto:official-tools:read",
        "tool.name.v0:read",
        "read",
        "tool.name.v0:view",
        "view",
    ] {
        assert_eq!(resolver.resolve(input).unwrap(), identity.stable_id);
    }
}

#[test]
fn resolver_rejects_duplicate_canonical_ids_and_alias_collisions() {
    let first = ToolIdentity::new(
        ToolSource::Uds,
        "uds:client-a",
        "weather",
        vec!["forecast".into()],
    );
    let duplicate = ToolIdentity::new(ToolSource::Uds, "uds:client-a", "weather", vec![]);
    let alias_collision = ToolIdentity::new(ToolSource::Uds, "uds:client-b", "forecast", vec![]);
    let mut resolver = ToolIdResolver::default();
    resolver.register(&first).unwrap();
    assert!(matches!(
        resolver.register(&duplicate),
        Err(ToolIdResolveError::Duplicate(_))
    ));
    assert!(matches!(
        resolver.register(&alias_collision),
        Err(ToolIdResolveError::Duplicate(_))
    ));
}

#[test]
fn resolver_reports_unknown_ids() {
    let resolver = ToolIdResolver::default();
    assert!(
        matches!(resolver.resolve("missing"), Err(ToolIdResolveError::Unknown(id)) if id == "missing")
    );
}

#[test]
fn stable_tool_ids_length_delimit_provider_id_to_avoid_colon_collisions() {
    let left = stable_tool_id(ToolSource::Uds, "a:b", "c");
    let right = stable_tool_id(ToolSource::Uds, "a", "b:c");

    assert_ne!(left, right);
    assert_eq!(left, "tool.v1:uds:3:a:b:c");
    assert_eq!(right, "tool.v1:uds:1:a:b:c");
}

#[test]
fn resolver_still_rejects_true_duplicate_ids_after_length_delimiting() {
    let first = ToolIdentity::new(ToolSource::Uds, "a:b", "c", vec![]);
    let duplicate = ToolIdentity::new(ToolSource::Uds, "a:b", "c", vec![]);
    let previously_colliding = ToolIdentity::new(ToolSource::Uds, "a", "b:c", vec![]);
    let mut resolver = ToolIdResolver::default();

    resolver.register(&first).unwrap();
    resolver.register(&previously_colliding).unwrap();
    assert!(matches!(
        resolver.register(&duplicate),
        Err(ToolIdResolveError::Duplicate(_))
    ));
}

#[test]
fn equivalent_policy_inputs_expand_raw_and_legacy_names() {
    assert_eq!(
        equivalent_policy_inputs("weather"),
        BTreeSet::from(["weather".to_string(), "tool.name.v0:weather".to_string()])
    );
    assert_eq!(
        equivalent_policy_inputs("tool.name.v0:weather"),
        BTreeSet::from(["weather".to_string(), "tool.name.v0:weather".to_string()])
    );
}
