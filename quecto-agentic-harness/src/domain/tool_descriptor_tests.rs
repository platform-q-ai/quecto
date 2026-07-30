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
