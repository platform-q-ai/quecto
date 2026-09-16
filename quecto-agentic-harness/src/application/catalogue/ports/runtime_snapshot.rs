//! The published runtime generation a model change consults (#1847): the
//! catalogue and the composed provider as one generation, or nothing before
//! the first successful composition. The application's own runtime snapshot
//! store implements it and the return type is the runtime capability's
//! snapshot, so today this port names the sibling rather than decoupling
//! from it; it becomes a real seam when runtime composition migrates
//! (#1849) and the store moves behind that capability.

use std::sync::Arc;

use crate::application::provider_runtime::CatalogueRuntimeSnapshot;

pub trait RuntimeSnapshotSource: Send + Sync {
    fn current_runtime(&self) -> Option<Arc<CatalogueRuntimeSnapshot>>;
}
