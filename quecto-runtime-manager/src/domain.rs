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
mod tests {
    use super::*;

    fn request() -> EnsureRuntimeRequest {
        EnsureRuntimeRequest {
            agent_profile_id: "agent-EANncutp8AIdZsDG5yZBsg".to_string(),
            user_id: Some("user".to_string()),
            project_id: "project-ahcu5C_pJUSBYiQLn7xzgw".to_string(),
            chat_id: "task-lDLtBzG0ERvp1jjTuVeuiA".to_string(),
            session_name: "session".to_string(),
            session_key: "key".to_string(),
            execution_model: None,
        }
    }

    #[test]
    fn runtime_ref_is_deterministic_safe_and_socket_friendly() {
        let body = request();
        let r = runtime_ref(&body);

        assert_eq!(r, runtime_ref(&body));
        assert!(r.len() <= 64);
        assert!(
            r.chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-'))
        );
        assert!(socket_path_within_uds_limit(Path::new("/data/sockets"), &r));
        assert_ne!(
            r,
            runtime_ref(&EnsureRuntimeRequest {
                chat_id: format!("{}-other", body.chat_id),
                ..body
            })
        );
    }

    #[test]
    fn validation_rejects_missing_identity_fields() {
        let mut body = request();
        body.session_name = " ".to_string();

        assert_eq!(
            validate_ensure_request(&body),
            Err("missing session_name".to_string())
        );
    }

    #[test]
    fn validation_accepts_pod_execution_model_for_background_board_runs() {
        let mut body = request();
        body.execution_model = Some("pod".to_string());

        assert_eq!(validate_ensure_request(&body), Ok(()));
    }

    #[test]
    fn validation_rejects_unknown_execution_model() {
        let mut body = request();
        body.execution_model = Some("docker".to_string());

        assert_eq!(
            validate_ensure_request(&body),
            Err("invalid execution_model docker".to_string())
        );
    }
}
