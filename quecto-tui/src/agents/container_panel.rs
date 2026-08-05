use std::collections::{BTreeMap, BTreeSet};

use crate::agents::roster::TrackedSubagent;
use crate::protocol::client::SubagentInfoEvent;

pub(crate) fn shared_container_uuid_set(
    tracked: &BTreeMap<String, TrackedSubagent<SubagentInfoEvent>>,
) -> BTreeSet<String> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for subagent in tracked.values() {
        if subagent.info.runtime_backend == "container" {
            if let Some(uuid) = subagent.info.container_uuid.clone() {
                *counts.entry(uuid).or_default() += 1;
            }
        }
    }
    counts
        .into_iter()
        .filter_map(|(uuid, count)| (count > 1).then_some(uuid))
        .collect()
}

pub(crate) fn shared_container_member_rows(
    tracked: &BTreeMap<String, TrackedSubagent<SubagentInfoEvent>>,
) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    for uuid in shared_container_uuid_set(tracked) {
        let mut ids: Vec<String> = tracked
            .iter()
            .filter(|(_, t)| t.info.container_uuid.as_deref() == Some(uuid.as_str()))
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        let last = ids.len().saturating_sub(1);
        for (idx, id) in ids.into_iter().enumerate() {
            rows.push((
                id,
                if idx == last {
                    "└ ".into()
                } else {
                    "├ ".into()
                },
            ));
        }
    }
    rows
}

pub(crate) fn is_shared_container(
    tracked: &BTreeMap<String, TrackedSubagent<SubagentInfoEvent>>,
    info: &SubagentInfoEvent,
) -> bool {
    info.container_uuid
        .as_deref()
        .is_some_and(|uuid| shared_container_uuid_set(tracked).contains(uuid))
}

pub(crate) fn environment_title(info: &SubagentInfoEvent) -> String {
    format!(
        "Env {} name:{} status:{} repo:{} branch:{} runtime:{} id:{} workspace:{} socket:{}",
        info.container_ref.as_deref().unwrap_or("?"),
        info.container_name.as_deref().unwrap_or("unknown"),
        info.environment_health
            .as_deref()
            .unwrap_or(info.status.as_str()),
        info.repo_url.as_deref().unwrap_or("unknown"),
        "unknown",
        info.runtime_backend.as_str(),
        info.environment_id
            .as_deref()
            .or(info.container_uuid.as_deref())
            .unwrap_or("unknown"),
        info.workspace_path.as_deref().unwrap_or("unknown"),
        info.socket_mode.as_deref().unwrap_or("unknown")
    )
}
