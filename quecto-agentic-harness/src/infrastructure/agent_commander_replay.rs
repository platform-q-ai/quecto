//! SPIKE: re-judge logged dry-run decisions with the current questions.
use std::path::Path;

use serde_json::Value;

use super::{AgentRole, CommanderEvent, ENDPOINT, Job, Worker, load_key};

/// Replays logged decisions (one session's records, in order) through the
/// current questions and policy, writing fresh records to `out_dir`. Each
/// record keeps the recent-events context it was originally judged with.
pub fn replay_session(records: &[Value], out_dir: &Path) -> Result<usize, String> {
    let first = records.first().ok_or("no records")?;
    let role_text = first["agent_role"]
        .as_str()
        .ok_or("record has no agent_role")?;
    let role = if role_text == "Root" {
        AgentRole::Root
    } else if role_text.starts_with("Child") {
        let parent_id = role_text
            .split_once("Some(\"")
            .and_then(|(_, rest)| rest.split_once("\")"))
            .map(|(id, _)| id.to_string());
        AgentRole::Child { parent_id }
    } else {
        return Err(format!("unknown agent_role {role_text}"));
    };
    let key = load_key().ok_or("no TypeSafe key")?;
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let mut worker = Worker::new(
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| e.to_string())?,
        key,
        out_dir.to_path_buf(),
        role,
        ENDPOINT.to_string(),
        std::time::Duration::from_millis(500),
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    for record in records {
        let event: CommanderEvent =
            serde_json::from_value(record["event"].clone()).map_err(|e| e.to_string())?;
        if let Some(recent) = record
            .pointer("/state_sent/recent_events")
            .and_then(Value::as_array)
        {
            worker.recent = recent
                .iter()
                .filter_map(|r| r.as_str().map(str::to_string))
                .collect();
        }
        runtime.block_on(
            worker.judge(Job {
                ts: record["ts"].as_str().unwrap_or_default().to_string(),
                seq: record["seq"].as_u64().unwrap_or_default(),
                session_key: record["session"]
                    .as_str()
                    .unwrap_or("no-session")
                    .to_string(),
                model: record["agent_model"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                event,
            }),
        );
    }
    Ok(records.len())
}
