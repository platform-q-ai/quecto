//! Contract tests for the `ContextSpillStore` port.

use quecto::domain::session::ContextSpillStore;

#[test]
fn port_is_object_safe() {
    fn _accepts_trait_object(_: &dyn ContextSpillStore) {}
}
