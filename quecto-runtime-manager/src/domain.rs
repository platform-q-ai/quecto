use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct EnsureRuntimeRequest {
    pub agent_profile_id: String,
    pub user_id: Option<String>,
    pub project_id: String,
    pub chat_id: String,
    pub session_name: String,
    pub session_key: String,
    pub execution_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<RepositoryCheckout>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<RuntimeCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<WorkflowExecution>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RepositoryCheckout {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RuntimeCapabilities {
    #[serde(default)]
    pub network: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<String>,
    #[serde(default)]
    pub github_cli: bool,
    #[serde(default)]
    pub git_write: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct WorkflowExecution {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_json: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_after_step_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeEnvelope {
    pub runtime_ref: String,
    pub status: String,
    pub labels: Vec<String>,
}

impl RuntimeEnvelope {
    pub fn running(runtime_ref: String) -> Self {
        Self {
            runtime_ref,
            status: "running".to_string(),
            labels: vec!["quecto".to_string(), "runtime-manager".to_string()],
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StopRuntimeResponse {
    pub runtime_ref: String,
    pub status: String,
    pub stopped: bool,
}

pub fn safe_part(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-') {
                ch
            } else {
                '-'
            }
        })
        .take(80)
        .collect()
}

pub fn runtime_ref(body: &EnsureRuntimeRequest) -> String {
    let full_key = format!(
        "{}:{}:{}",
        body.agent_profile_id, body.project_id, body.chat_id
    );
    let digest = hex_16(&Sha256::digest(full_key.as_bytes()));
    let agent = safe_part(&body.agent_profile_id)
        .chars()
        .take(12)
        .collect::<String>();
    let project = safe_part(&body.project_id)
        .chars()
        .take(12)
        .collect::<String>();
    let chat = safe_part(&body.chat_id)
        .chars()
        .take(12)
        .collect::<String>();
    format!("cc-{agent}-{project}-{chat}-{digest}")
        .chars()
        .take(64)
        .collect()
}

pub fn validate_ensure_request(body: &EnsureRuntimeRequest) -> Result<(), String> {
    for (key, value) in [
        ("agent_profile_id", &body.agent_profile_id),
        ("project_id", &body.project_id),
        ("chat_id", &body.chat_id),
        ("session_name", &body.session_name),
        ("session_key", &body.session_key),
    ] {
        if value.trim().is_empty() {
            return Err(format!("missing {key}"));
        }
    }

    if let Some(execution_model) = body.execution_model.as_deref() {
        if !matches!(execution_model, "process" | "pod") {
            return Err(format!("invalid execution_model {execution_model}"));
        }
    }

    Ok(())
}

pub fn socket_path_within_uds_limit(socket_root: &Path, runtime_ref: &str) -> bool {
    socket_root
        .join(format!("{runtime_ref}.sock"))
        .to_string_lossy()
        .len()
        < 104
}

fn hex_16(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(16);
    for byte in bytes.iter().take(8) {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
#[path = "domain_tests.rs"]
mod tests;
