use crate::domain::agent_launch_backend::ParentEndpoint;

/// Connect to the typed parent endpoint with retries and exponential backoff.
pub(crate) async fn connect_endpoint_with_retry(
    endpoint: &ParentEndpoint,
    max_retries: u32,
) -> Option<tokio::net::UnixStream> {
    let mut delay_ms = 50u64;
    for _ in 0..max_retries {
        if let Ok(stream) = super::parent_endpoint::connect(endpoint).await {
            return Some(stream);
        }
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        delay_ms = (delay_ms * 2).min(500); // Cap at 500ms
    }
    None
}
