//! Contract coverage for model limit source ports.

use quecto::catalogue_limits_app::{ModelLimitSource, ResolveModelLimitsUseCase};
use quecto::domain::catalogue::ModelRef;

struct Limits;

impl ModelLimitSource for Limits {
    fn limits_for(&self, reference: &ModelRef) -> (Option<u32>, Option<usize>) {
        assert_eq!(reference.qualified_id(), "provider/model");
        (Some(256), Some(1024))
    }
}

#[test]
fn model_limit_source_receives_typed_model_references() {
    assert_eq!(
        ResolveModelLimitsUseCase::new().resolve(&Limits, "provider/model"),
        (Some(256), Some(1024))
    );
}

#[test]
fn invalid_model_reference_short_circuits_without_calling_source() {
    assert_eq!(
        ResolveModelLimitsUseCase::new().resolve(&Limits, "bare-model"),
        (None, None)
    );
}
