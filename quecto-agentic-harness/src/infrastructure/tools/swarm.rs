//! Opt-in #1680 spike. No second execution engine: Python Lab owns processes.
use super::python_lab::PythonLabTool;
use crate::domain::{
    error::DomainError,
    tool::{Tool, ToolDefinition, ToolResult},
};
use std::{future::Future, pin::Pin};

pub struct SwarmTool {
    inner: PythonLabTool,
}
impl SwarmTool {
    pub fn new(inner: PythonLabTool) -> Self {
        Self { inner }
    }
}
impl Tool for SwarmTool {
    fn definition(&self) -> ToolDefinition {
        let mut definition = self.inner.definition();
        definition.name = "swarm".into();
        definition.description = "Container-only experimental swarm. Python Lab execution/output/cancel semantics; packaged `swarm` Python module is preloaded. Shared SQLite is durable outside execution artifacts. Reservations/identity are cooperative, not security. Use code, not path, in this spike.".into();
        definition
    }
    fn set_session_key(&self, key: String) {
        self.inner.set_session_key(key);
    }
    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let arguments = arguments.to_string();
        Box::pin(async move {
            if let Err(message) = require_container() {
                return error(message);
            }
            let mut value: serde_json::Value = match serde_json::from_str(&arguments) {
                Ok(value) => value,
                Err(e) => return error(format!("invalid JSON: {e}")),
            };
            if value.get("op").and_then(|v| v.as_str()).unwrap_or("run") == "run" {
                if value.get("path").is_some() {
                    return error("spike supports code only, not path".into());
                }
                let Some(code) = value.get("code").and_then(|v| v.as_str()) else {
                    return error("code required".into());
                };
                let source = serde_json::to_string(include_str!("swarm_python/swarm.py")).unwrap();
                value["code"] = format!("import sys, types\nswarm = types.ModuleType('swarm')\nsys.modules['swarm'] = swarm\nexec({source}, swarm.__dict__)\n{code}").into();
            }
            self.inner.execute(&value.to_string()).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::security::sandbox::Sandbox;
    use std::sync::Arc;
    #[tokio::test]
    async fn packaged_helper_loads_under_isolated_python() {
        let dir = tempfile::tempdir().unwrap();
        let tool = SwarmTool::new(PythonLabTool::new(
            Arc::new(dir.path().into()),
            Arc::new(Sandbox::new(None)),
            Default::default(),
        ));
        assert_eq!(tool.definition().name, "swarm");
        if require_container().is_err() {
            assert!(
                tool.execute(r#"{"op":"run","code":"print('must not run')"}"#)
                    .await
                    .unwrap()
                    .is_error
            );
            return;
        }
        let result = tool
            .execute(r#"{"op":"run","code":"import swarm; print('packaged', swarm.__name__)"}"#)
            .await
            .unwrap();
        assert!(!result.is_error, "{}", result.content);
        assert!(
            result.content.contains("packaged swarm"),
            "{}",
            result.content
        );
    }
}

/// Deliberately narrow Linux recognition: kernel mount records, not the presence
/// of a writable marker. This is not malicious same-user authentication.
pub fn recognized_container_mounts(mounts: &str) -> bool {
    mounts.lines().any(|line| {
        let fields: Vec<_> = line.split_whitespace().collect();
        fields.len() > 6
            && ((fields[4] == "/run/.containerenv"
                && fields[3].contains("/containers/overlay-containers/"))
                || (fields[4] == "/etc/hostname"
                    && fields[3].contains("/containers/")
                    && fields[3].ends_with("/hostname")))
    })
}
pub fn require_container() -> Result<(), String> {
    let mounts = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    if recognized_container_mounts(&mounts) {
        Ok(())
    } else {
        Err("swarm spike requires a recognized Linux Docker/Podman container (kernel mountinfo); launch in a container with its own /etc/hostname mount".into())
    }
}
pub fn enabled() -> bool {
    std::env::var_os("QUECTO_SWARM_SPIKE").is_some()
}
fn error(message: String) -> Result<ToolResult, DomainError> {
    Ok(ToolResult {
        content: message,
        is_error: true,
        image_blocks: vec![],
        delivery_metadata: None,
    })
}

#[cfg(test)]
mod container_tests {
    use super::*;
    #[test]
    fn kernel_mount_classifier_fails_closed() {
        assert!(!recognized_container_mounts(""));
        assert!(!recognized_container_mounts(
            "1 2 0:1 / /run/.containerenv rw - tmpfs tmpfs rw"
        ));
        assert!(!recognized_container_mounts(
            "1 2 0:1 / / rw - ext4 /dev/root rw"
        ));
        assert!(recognized_container_mounts(
            "1 2 0:1 /var/lib/docker/containers/abc/hostname /etc/hostname rw - ext4 /dev/root rw"
        ));
        assert!(recognized_container_mounts(
            "1 2 0:1 /containers/overlay-containers/abc/userdata/.containerenv /run/.containerenv rw - tmpfs tmpfs rw"
        ));
    }
}
