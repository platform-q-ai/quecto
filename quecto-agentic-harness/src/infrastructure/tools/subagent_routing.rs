use std::collections::HashSet;
use std::path::PathBuf;

use crate::domain::session::SubagentLiveness;

use super::subagent_registry::{SubagentRegistry, resolve_registry_key};

pub const MAX_INSPECTION_ROUTE_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InspectionRoute {
    Direct {
        socket_path: PathBuf,
    },
    ViaAncestor {
        ancestor_id: String,
        ancestor_socket_path: PathBuf,
        target_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutableInspectionCommand {
    GetMessages,
    GetMessagesTail,
    GetMessage,
    Sync,
    GetState,
}

pub const UDS_INSPECTION_ALLOWLIST: &[RoutableInspectionCommand] = &[
    RoutableInspectionCommand::GetMessages,
    RoutableInspectionCommand::GetMessagesTail,
    RoutableInspectionCommand::GetMessage,
    RoutableInspectionCommand::Sync,
    RoutableInspectionCommand::GetState,
];

impl RoutableInspectionCommand {
    pub fn from_uds_type(command_type: &str) -> Option<Self> {
        match command_type {
            "get_messages" => Some(Self::GetMessages),
            "get_messages_tail" => Some(Self::GetMessagesTail),
            "get_message" => Some(Self::GetMessage),
            "sync" => Some(Self::Sync),
            "get_state" => Some(Self::GetState),
            _ => None,
        }
    }

    pub fn from_agent_cmd(command: &str) -> Option<Self> {
        match command {
            "get_messages" => Some(Self::GetMessages),
            "get_state" => Some(Self::GetState),
            _ => None,
        }
    }
}

pub fn resolve_inspection_route(
    registry: &SubagentRegistry,
    target_ref: &str,
) -> Result<InspectionRoute, String> {
    let entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    let target_key = resolve_registry_key(&entries, target_ref).map_err(|err| match err {
        crate::domain::subagent::DisplayNameResolveError::NoLiveMatch { display_name } => {
            format!("no live subagent named '{display_name}' (not found)")
        }
        crate::domain::subagent::DisplayNameResolveError::AmbiguousLiveMatch { display_name } => {
            format!("duplicate live subagent display label '{display_name}'")
        }
    })?;
    let target = entries
        .get(&target_key)
        .ok_or_else(|| format!("subagent '{}' not found in registry", target_ref))?;
    if target.persisted_liveness != SubagentLiveness::Live {
        return Err(format!(
            "subagent '{target_ref}' is {} and has no live inspection route",
            match target.persisted_liveness {
                SubagentLiveness::Live => "live",
                SubagentLiveness::Detached => "detached",
                SubagentLiveness::Dead => "dead",
            }
        ));
    }
    if !target.socket_path.as_os_str().is_empty() {
        return Ok(InspectionRoute::Direct {
            socket_path: target.socket_path.clone(),
        });
    }

    let mut visited = HashSet::new();
    visited.insert(target_key.clone());
    let mut current = target.parent_id.clone();
    for _ in 0..MAX_INSPECTION_ROUTE_DEPTH {
        let Some(parent_ref) = current else {
            return Err(non_connectable_error(target_ref));
        };
        let parent_key = if entries.contains_key(&parent_ref) {
            parent_ref.clone()
        } else {
            resolve_registry_key(&entries, &parent_ref)
                .map_err(|_| format!("subagent '{target_ref}' is listed but its parent '{parent_ref}' is missing or ambiguous; no ancestor-connectable socket is available"))?
        };
        if !visited.insert(parent_key.clone()) {
            return Err(format!(
                "subagent '{target_ref}' ancestor chain contains a cycle; no safe inspection route is available"
            ));
        }
        let parent = entries
            .get(&parent_key)
            .ok_or_else(|| format!("subagent '{target_ref}' parent '{parent_ref}' not found; no ancestor-connectable socket is available"))?;
        if !parent.socket_path.as_os_str().is_empty() {
            return Ok(InspectionRoute::ViaAncestor {
                ancestor_id: parent.effective_display_name(&parent_key).to_string(),
                ancestor_socket_path: parent.socket_path.clone(),
                target_id: target_key.clone(),
            });
        }
        current = parent.parent_id.clone();
    }
    Err(format!(
        "subagent '{target_ref}' ancestor chain exceeds maximum inspection route depth ({MAX_INSPECTION_ROUTE_DEPTH}); no safe route is available"
    ))
}

pub fn non_connectable_error(agent_id: &str) -> String {
    format!(
        "subagent '{agent_id}' is listed but has no ancestor-connectable socket (nested container descendant sockets are not reachable from this session)"
    )
}
