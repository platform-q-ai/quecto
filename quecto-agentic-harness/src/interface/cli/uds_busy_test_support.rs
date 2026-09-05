//! Test-support entry points for the reader-task busy interceptor
//! (`uds_busy_subagents`), used by `uds_subagent_liveness.feature`. Built
//! only under `cfg(test)` or the `test-support` feature.

/// Test-support: run one raw command line through the reader-task busy
/// interceptor for sub-agent liveness commands, with no sub-agent registry.
/// Returns whether it was handled off the dispatch loop and, when handled,
/// the correlated response written to the client's channel.
pub async fn busy_reader_intercept(line: &str) -> (bool, Option<serde_json::Value>) {
    let clients = super::uds_ext_protocol::new_client_tool_registry();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(8);
    super::uds_ext_protocol::register_client_writer(&clients, 1, tx);
    let (broadcast_tx, _broadcast_rx) = tokio::sync::broadcast::channel::<String>(8);
    let handled =
        super::uds_busy_subagents::intercept(super::uds_busy_subagents::BusySubagentCtx {
            line,
            subagents: &None,
            broadcast_tx: &broadcast_tx,
            clients: &clients,
            client_id: 1,
        })
        .await;
    if !handled {
        return (false, None);
    }
    let response = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .ok()
        .flatten()
        .and_then(|l| serde_json::from_str(&l).ok());
    (true, response)
}

/// Test-support: like [`busy_reader_intercept`] but against a sub-agent
/// registry pre-seeded with `names` (pid 0, no monitors). Returns whether the
/// command was handled off the dispatch loop, the correlated response, and
/// how many registry entries remain afterwards (#1626).
pub async fn busy_reader_intercept_with_registry(
    line: &str,
    names: &[&str],
) -> (bool, Option<serde_json::Value>, usize) {
    use crate::infrastructure::tools::subagent_registry::SubagentEntry;
    let registry = crate::infrastructure::tools::subagent_registry::new_registry();
    for name in names {
        registry.lock().unwrap().insert(
            (*name).to_string(),
            SubagentEntry::new(std::path::PathBuf::from(format!("/tmp/{name}.sock")), 0),
        );
    }
    let (broadcast_tx, _broadcast_rx) = tokio::sync::broadcast::channel::<String>(8);
    let clients = super::uds_ext_protocol::new_client_tool_registry();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(8);
    super::uds_ext_protocol::register_client_writer(&clients, 1, tx);
    let handled =
        super::uds_busy_subagents::intercept(super::uds_busy_subagents::BusySubagentCtx {
            line,
            subagents: &Some(registry.clone()),
            broadcast_tx: &broadcast_tx,
            clients: &clients,
            client_id: 1,
        })
        .await;
    let remaining = registry.lock().unwrap().len();
    if !handled {
        return (false, None, remaining);
    }
    let response = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .ok()
        .flatten()
        .and_then(|l| serde_json::from_str(&l).ok());
    (true, response, remaining)
}
