//! Contract tests for the `ToolRegistry` port.

use quecto::domain::tool::ToolRegistry;

#[test]
fn port_is_object_safe() {
    fn _accepts_trait_object(_: &dyn ToolRegistry) {}
}
