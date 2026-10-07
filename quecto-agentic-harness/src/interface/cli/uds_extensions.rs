//! Tool catalogue snapshot helpers for UDS control/query clients, and the
//! configured extensions' edge (#2446): launched once the socket listens,
//! their connections recognised at accept, their registrations reported.
use std::path::Path;
use std::sync::Arc;

use crate::domain::agents::entities::configured_extensions::AgentExtensions;
use crate::infrastructure::processes::configured_extensions::{ConfiguredExtensions, plan};
use crate::infrastructure::processes::owned_child_supervisor::OwnedChildSupervisor;

pub(super) type ToolCatalogueSnapshot = std::sync::Arc<tokio::sync::RwLock<Vec<serde_json::Value>>>;

pub(super) type ExtRegistry = std::sync::Arc<
    std::sync::Mutex<crate::infrastructure::extensions::registry::ExtensionRegistry>,
>;

/// The running configured extensions of this agent; `None` when it has none.
pub(crate) type Extensions = Option<Arc<ConfiguredExtensions>>;

/// Launch `extensions` for an agent now listening on `socket`.
pub(super) fn launch(extensions: &AgentExtensions, socket: &Path, base_dir: &Path) -> Extensions {
    match extensions.specs.as_slice() {
        [] => None,
        [_, ..] => Some(Arc::new(ConfiguredExtensions::launch(
            plan(extensions, socket, base_dir),
            OwnedChildSupervisor::process_wide(),
        ))),
    }
}

/// At accept: record client `client_id` as the extension its peer process
/// belongs to, if it is one.
pub(super) fn claim_connection(
    extensions: &Extensions,
    stream: &tokio::net::UnixStream,
    registry: &super::uds_ext_protocol::ClientToolRegistry,
    client_id: u64,
) {
    let peer = stream
        .peer_cred()
        .ok()
        .and_then(|credentials| credentials.pid());
    let (Some(extensions), Some(peer)) = (extensions, peer) else {
        return;
    };
    let mut clients = registry.lock().unwrap_or_else(|e| e.into_inner());
    let state = clients.entry(client_id).or_default();
    match extensions.claim(Some(peer)) {
        Some(claim) => {
            tracing::info!(client_id, extension = %claim.name(), "configured extension connected");
            state.extension = Some(claim);
        }
        None => state.extension_candidate = Some((Arc::clone(extensions), peer)),
    }
}

/// At `register_tools`: a connection not claimed at accept (it connected
/// before its extension's launch was recorded) is claimed now if its peer
/// is an extension's, once any launch in flight has been recorded.
pub(super) async fn claim_late(
    registry: &super::uds_ext_protocol::ClientToolRegistry,
    client_id: u64,
) {
    let candidate = {
        let clients = registry.lock().unwrap_or_else(|e| e.into_inner());
        clients
            .get(&client_id)
            .and_then(|state| state.extension_candidate.clone())
    };
    let Some((extensions, peer)) = candidate else {
        return;
    };
    extensions.launched().await;
    let claim = extensions.claim(Some(peer));
    let mut clients = registry.lock().unwrap_or_else(|e| e.into_inner());
    if let (Some(claim), Some(state)) = (claim, clients.get_mut(&client_id)) {
        tracing::info!(client_id, extension = %claim.name(), "configured extension claimed late");
        state.extension_candidate = None;
        state.extension = Some(claim);
    }
}

/// The configured extension client `client_id` is, by name.
pub(super) fn extension_of(
    registry: &super::uds_ext_protocol::ClientToolRegistry,
    client_id: u64,
) -> Option<String> {
    let clients = registry.lock().unwrap_or_else(|e| e.into_inner());
    clients
        .get(&client_id)
        .and_then(|state| state.extension.as_ref())
        .map(|claim| claim.name())
}

/// Client `client_id`'s `register_tools` succeeded: if it is an
/// extension's connection, its tools are in.
pub(super) fn tools_registered(
    registry: &super::uds_ext_protocol::ClientToolRegistry,
    client_id: u64,
) {
    let clients = registry.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(claim) = clients
        .get(&client_id)
        .and_then(|state| state.extension.as_ref())
    {
        claim.tools_registered();
    }
}

/// How many connected clients are configured extensions: the agent owns
/// them, so they never keep it alive past its last real client.
pub(super) fn extension_clients(registry: &super::uds_ext_protocol::ClientToolRegistry) -> u32 {
    let clients = registry.lock().unwrap_or_else(|e| e.into_inner());
    let owned = clients
        .values()
        .filter(|state| state.extension.is_some())
        .count();
    u32::try_from(owned).unwrap_or(u32::MAX)
}
