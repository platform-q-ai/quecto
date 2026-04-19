//! Contract tests for the `AuditSink` port.

use quecto::domain::audit::AuditSink;

#[test]
fn port_is_object_safe() {
    fn _accepts_trait_object(_: &dyn AuditSink) {}
}
