//! The published runtime generation a model change consults (#1847): the
//! catalogue and the composed provider as one generation, or nothing before
//! the first successful composition. The runtime snapshot store implements
//! it; the use case applies the selection rule itself.

use std::sync::Arc;

use crate::application::provider_runtime::CatalogueRuntimeSnapshot;

pub trait RuntimeSnapshotSource: Send + Sync {
    fn current_runtime(&self) -> Option<Arc<CatalogueRuntimeSnapshot>>;
}
