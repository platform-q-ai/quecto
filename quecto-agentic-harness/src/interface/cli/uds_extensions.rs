//! Tool catalogue snapshot helpers for UDS control/query clients.

pub(super) type ToolCatalogueSnapshot = std::sync::Arc<tokio::sync::RwLock<Vec<serde_json::Value>>>;
