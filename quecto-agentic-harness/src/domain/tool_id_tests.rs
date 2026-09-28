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

/// Round-trip: every id `stable_tool_id` mints parses back to its parts
/// (#2247 round 2 L1/L2), including a provider id carrying `:` and `.`.
#[test]
fn a_minted_stable_id_parses_back_to_its_parts() {
    for (source, provider_id, name) in [
        (ToolSource::BundledNative, "quecto:official-tools", "bash"),
        (ToolSource::Uds, "uds:client-a", "weather_v2"),
        (ToolSource::Runtime, "com.example.ext", "fetch-page"),
    ] {
        let id = stable_tool_id(source, provider_id, name);
        assert_eq!(
            parse_stable_tool_id(&id),
            Some(StableToolId {
                source,
                provider_id,
                name
            }),
            "{id}"
        );
    }
}

/// The grammar is an allowlist: anything but
/// `tool.v1:<source>:<len>:<provider of len bytes>:<name>` with a known
/// source and a name of `[A-Za-z0-9_-]` is not a stable id.
#[test]
fn only_the_stable_id_grammar_parses() {
    for id in [
        "",
        "bash",
        "tool.name.v0:bash",
        "tool.v2:bundled-native:21:quecto:official-tools:bash",
        "tool.v1:plugin:21:quecto:official-tools:bash",
        "tool.v1:bundled-native:22:quecto:official-tools:bash",
        "tool.v1:bundled-native:20:quecto:official-tools:bash",
        "tool.v1:bundled-native:+21:quecto:official-tools:bash",
        "tool.v1:bundled-native::quecto:official-tools:bash",
        "tool.v1:bundled-native:0::bash",
        "tool.v1:bundled-native:99999999999999999999999:x:bash",
        "tool.v1:bundled-native:21:quecto:official-tools:",
        "tool.v1:bundled-native:21:quecto:official-tools:ba.sh",
        "tool.v1:bundled-native:21:quecto:official-tools:ba sh",
        "tool.v1:bundled-native:21:quecto:official-tools:bash:x",
        "tool.v1:bundled-native:21:quecto:official-tools",
        "tool.v1:bundled-native:3:é:bash",
        "tool.v1:bundled-native:1:é:bash",
        // Only the canonical length text `stable_tool_id` mints: no leading
        // zero, sign or blank (#2247 review F1).
        "tool.v1:bundled-native:021:quecto:official-tools:bash",
        "tool.v1:bundled-native:0021:quecto:official-tools:bash",
        "tool.v1:bundled-native:03:web:web_search",
        "tool.v1:bundled-native: 21:quecto:official-tools:bash",
        "tool.v1:bundled-native:-21:quecto:official-tools:bash",
        "tool.v1:bundled-native:0:x:bash",
    ] {
        assert_eq!(parse_stable_tool_id(id), None, "{id:?}");
    }
}

#[test]
fn a_tool_source_parses_only_from_its_own_label() {
    for source in [
        ToolSource::BundledNative,
        ToolSource::Uds,
        ToolSource::Runtime,
    ] {
        assert_eq!(ToolSource::parse(source.as_str()), Some(source));
    }
    for label in ["", "bundled", "UDS", "runtime ", "plugin"] {
        assert_eq!(ToolSource::parse(label), None, "{label:?}");
    }
}
