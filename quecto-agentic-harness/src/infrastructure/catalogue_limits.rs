//! Infrastructure adapter for catalogue-backed model runtime limits.

use std::path::{Path, PathBuf};

use crate::catalogue_limits_app::ModelLimitSource;
use crate::domain::catalogue::ModelRef;
use crate::infrastructure::model_registry::ModelRegistry;

#[derive(Debug, Clone)]
pub struct ModelRegistryLimitSource {
    base_dir: PathBuf,
}

impl ModelRegistryLimitSource {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    pub fn from_base_dir(base_dir: &Path) -> Self {
        Self::new(base_dir.to_path_buf())
    }

    fn registry(&self) -> ModelRegistry {
        ModelRegistry::load_from_path(&self.base_dir.join("models.json"))
            .unwrap_or_else(|_| ModelRegistry::builtin())
    }
}

impl ModelLimitSource for ModelRegistryLimitSource {
    fn limits_for(&self, reference: &ModelRef) -> (Option<u32>, Option<usize>) {
        let registry = self.registry();
        let qualified = reference.qualified_id();
        (
            registry.max_tokens_for(&qualified),
            registry.context_window_for(&qualified),
        )
    }
}

pub fn model_limits_from_base_dir(
    base_dir: &Path,
    qualified_model: &str,
) -> (Option<u32>, Option<usize>) {
    crate::catalogue_limits_app::ResolveModelLimitsUseCase::new().resolve(
        &ModelRegistryLimitSource::from_base_dir(base_dir),
        qualified_model,
    )
}

#[cfg(test)]
#[path = "catalogue_limits_tests.rs"]
mod tests;
