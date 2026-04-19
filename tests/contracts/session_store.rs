//! Contract tests for the `SessionStore` port.

use quecto::domain::session::SessionStore;

#[test]
fn port_is_object_safe() {
    fn _accepts_trait_object(_: &dyn SessionStore) {}
}
