//! Test-support entry points for the reader-task busy interceptor
//! (`uds_busy_subagents`), used by `uds_subagent_liveness.feature` and the
//! fleet teardown feature (#1938). Built only under `cfg(test)` or the
//! `test-support` feature.

use std::sync::Arc;

use crate::application::subagents::use_cases::TerminateAllDelegatedAgents;
use crate::infrastructure::tools::subagent_registry::SubagentRegistry;

/// Run one raw command line through the reader-task busy interceptor with no
/// sub-agent registry. Returns whether it was handled off the dispatch loop
/// and, when handled, the correlated response written to the client's channel.
pub async fn busy_reader_intercept(line: &str) -> (bool, Option<serde_json::Value>) {
    run_intercept(line, &None, None).await
}

/// Like [`busy_reader_intercept`] against the caller's registry and fleet
/// teardown (#1938): what a busy-path `delete_all_subagents` invokes.
pub async fn busy_reader_intercept_with_fleet(
    line: &str,
    registry: SubagentRegistry,
    fleet: Option<Arc<TerminateAllDelegatedAgents>>,
) -> (bool, Option<serde_json::Value>) {
    run_intercept(line, &Some(registry), fleet.as_ref()).await
}

async fn run_intercept(
    line: &str,
    subagents: &Option<SubagentRegistry>,
    fleet: Option<&Arc<TerminateAllDelegatedAgents>>,
) -> (bool, Option<serde_json::Value>) {
    let clients = super::uds_ext_protocol::new_client_tool_registry();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(8);
    super::uds_ext_protocol::register_client_writer(&clients, 1, tx);
    let handled =
        super::uds_busy_subagents::intercept(super::uds_busy_subagents::BusySubagentCtx {
            line,
            subagents,
            fleet,
            clients: &clients,
            client_id: 1,
        })
        .await;
    if !handled {
        return (false, None);
    }
    // A busy-path delete answers from a detached task once the fleet has
    // settled: bounded by the fleet's own budgets plus slack.
    let response = tokio::time::timeout(std::time::Duration::from_secs(30), rx.recv())
        .await
        .ok()
        .flatten()
        .and_then(|l| serde_json::from_str(&l).ok());
    (true, response)
}
