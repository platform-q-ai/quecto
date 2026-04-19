//! Contract tests for the `LlmProvider` port.

use quecto::domain::provider::LlmProvider;

#[test]
fn port_is_object_safe() {
    fn _accepts_trait_object(_: &dyn LlmProvider) {}
}
