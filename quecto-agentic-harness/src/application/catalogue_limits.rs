//! Application use case for model-specific runtime limits.
//!
//! Interface adapters should not load catalogue files to infer output/context
//! limits. They pass stable model references to this use case; infrastructure
//! supplies the concrete catalogue-backed lookup.

use crate::domain::catalogue::ModelRef;

pub trait ModelLimitSource {
    fn limits_for(&self, reference: &ModelRef) -> (Option<u32>, Option<usize>);
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ResolveModelLimitsUseCase;

impl ResolveModelLimitsUseCase {
    pub fn new() -> Self {
        Self
    }

    pub fn resolve<S: ModelLimitSource>(
        &self,
        source: &S,
        qualified_model: &str,
    ) -> (Option<u32>, Option<usize>) {
        let Ok(reference) = ModelRef::parse_qualified(qualified_model) else {
            return (None, None);
        };
        source.limits_for(&reference)
    }
}

#[cfg(test)]
#[path = "catalogue_limits_tests.rs"]
mod tests;
