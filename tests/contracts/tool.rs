//! Contract tests for the `Tool` port.

use quecto::domain::tool::Tool;

#[test]
fn port_is_object_safe() {
    fn _accepts_trait_object(_: &dyn Tool) {}
}
