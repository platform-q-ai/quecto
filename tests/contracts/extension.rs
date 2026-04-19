//! Contract tests for the `Extension` port.

use quecto::domain::extension::Extension;

#[test]
fn port_is_object_safe() {
    fn _accepts_trait_object(_: &dyn Extension) {}
}
