use crate::domain::agent_launch_backend::ParentEndpoint;
use crate::domain::error::DomainError;
use std::time::Duration;

#[cfg(test)]
pub(crate) async fn wait_for_socket(path: &std::path::Path) -> Result<(), DomainError> {
    super::parent_endpoint::wait_ready(
        &ParentEndpoint::DirectUds(path.to_path_buf()),
        Duration::from_secs(10),
    )
    .await
}

pub(crate) async fn wait_for_endpoint_or_child_exit(
    endpoint: &ParentEndpoint,
    child: &mut tokio::process::Child,
) -> Result<(), DomainError> {
    tokio::select! {
        socket_result = super::parent_endpoint::wait_ready(endpoint, Duration::from_secs(10)) => socket_result,
        child_status = child.wait() => {
            let detail = match child_status {
                Ok(status) => format!(" with status {status}"),
                Err(error) => format!(": failed to observe exit status: {error}"),
            };
            Err(DomainError::Tool(format!(
                "subagent exited before socket ready{}",
                detail
            )))
        }
    }
}
