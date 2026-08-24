use super::*;

struct FakeLimitSource;

impl ModelLimitSource for FakeLimitSource {
    fn limits_for(
        &self,
        reference: &crate::domain::catalogue::ModelRef,
    ) -> (Option<u32>, Option<usize>) {
        assert_eq!(reference.qualified_id(), "provider/model");
        (Some(123), Some(456))
    }
}

#[test]
fn resolve_model_limits_rejects_unqualified_models_without_touching_source() {
    assert_eq!(
        ResolveModelLimitsUseCase::new().resolve(&FakeLimitSource, "not-qualified"),
        (None, None)
    );
}

#[test]
fn resolve_model_limits_passes_typed_reference_to_source() {
    assert_eq!(
        ResolveModelLimitsUseCase::new().resolve(&FakeLimitSource, "provider/model"),
        (Some(123), Some(456))
    );
}
