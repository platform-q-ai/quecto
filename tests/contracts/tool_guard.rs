//! Contract tests for the `ToolGuard` port.

use quecto::domain::tool::ToolGuard;

#[test]
fn port_is_object_safe() {
    fn _accepts_trait_object(_: &dyn ToolGuard) {}
}
