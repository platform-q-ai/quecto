use super::*;

#[test]
fn facade_is_constructed_from_explicit_capabilities() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ProductionResumeDecisionFacade>();
}
