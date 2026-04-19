//! Contract tests for the `OnboardStore` port.

use quecto::domain::workspace::OnboardStore;

#[test]
fn port_is_object_safe() {
    fn _accepts_trait_object(_: &dyn OnboardStore) {}
}
