//! The sub-agent roster presented on the wire (#524): the registry's entries
//! as sorted [`SubagentInfo`] rows, for snapshots and live broadcasts.
use crate::interface::uds::wire::protocol::SubagentInfo;

/// Build a sorted list of [`SubagentInfo`] from the shared registry.
///
/// Acquires the registry mutex briefly and maps each entry to the protocol type.
/// Returns an empty vec if the registry is `None` or empty.
pub fn build_subagent_info_list(
    registry: &Option<crate::infrastructure::tools::agents::subagent_registry::SubagentRegistry>,
) -> Vec<SubagentInfo> {
    build_subagent_info_list_filtered(registry, |_| true)
}

/// Build a sorted list of live/surviving [`SubagentInfo`] rows for authoritative
/// live roster broadcasts.
///
/// Retained dead/exited tombstones remain visible through [`build_subagent_info_list`]
/// for existing snapshot/query contracts, but live `subagent_state_changed`
/// broadcasts must not resurrect them in clients that treat the event as the
/// root roster snapshot.
pub fn build_live_subagent_info_list(
    registry: &Option<crate::infrastructure::tools::agents::subagent_registry::SubagentRegistry>,
) -> Vec<SubagentInfo> {
    build_subagent_info_list_filtered(registry, |effective| {
        effective.persisted_liveness
            == crate::domain::sessions::entities::session::SubagentLiveness::Live
            && *effective.status
                != crate::infrastructure::tools::agents::subagent_registry::SubagentStatus::Exited
    })
}

fn build_subagent_info_list_filtered(
    registry: &Option<crate::infrastructure::tools::agents::subagent_registry::SubagentRegistry>,
    include: impl Fn(&SubagentInfoBuildEntry<'_>) -> bool,
) -> Vec<SubagentInfo> {
    let Some(reg) = registry else {
        return Vec::new();
    };
    let mut list: Vec<SubagentInfo> = {
        let guard = reg.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .iter()
            .filter_map(|(id, entry)| {
                let status =
                    crate::infrastructure::tools::agents::subagent_registry::effective_status(&guard, id)
                        .unwrap_or_else(|| entry.status.clone());
                if !include(&SubagentInfoBuildEntry {
                    persisted_liveness: entry.persisted_liveness,
                    status: &status,
                }) {
                    return None;
                }
                let display_name = entry.effective_display_name(id).to_string();
                let environment =
                    crate::infrastructure::tools::agents::subagent_environment_wire::environment_wire(
                        entry,
                    );
                let execution_backend =
                    crate::infrastructure::tools::agents::subagent_environment_wire::execution_backend(
                        entry,
                        environment.as_ref(),
                    );
                Some(SubagentInfo {
                    agent_id: display_name.clone(),
                    agent_uuid: Some(entry.agent_uuid.to_string()),
                    display_name: Some(display_name),
                    status: status.to_wire_str().to_string(),
                    liveness: Some(match entry.persisted_liveness {
                        crate::domain::sessions::entities::session::SubagentLiveness::Live => {
                            "live".to_string()
                        }
                        crate::domain::sessions::entities::session::SubagentLiveness::Detached => {
                            "detached".to_string()
                        }
                        crate::domain::sessions::entities::session::SubagentLiveness::Dead => {
                            "dead".to_string()
                        }
                    }),
                    last_tool: entry.last_tool.clone(),
                    last_error: entry.last_error.clone(),
                    pid: entry.pid,
                    socket_path: None,
                    parent_id: entry.parent_id.clone(),
                    workflow: entry.workflow.clone(),
                    read_only: entry.read_only,
                    execution_backend,
                    environment,
                    launch_generation: entry
                        .launch_generation
                        .or(entry.reported_generation)
                        .map(|generation| generation.get()),
                })
            })
            .collect()
    }; // guard dropped here — sort happens outside critical section
    list.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
    list
}

struct SubagentInfoBuildEntry<'a> {
    persisted_liveness: crate::domain::sessions::entities::session::SubagentLiveness,
    status: &'a crate::infrastructure::tools::agents::subagent_registry::SubagentStatus,
}

#[cfg(test)]
mod environment_tests;

#[cfg(test)]
mod effective_status_tests;
