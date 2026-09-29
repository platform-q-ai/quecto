//! #2281: the recovery scenarios' board helpers: structured ops and the
//! worker's claim through them, the harness-only admission calls, and what
//! a board poll treats as contention.

use super::context;
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;

/// The worker's claim as structured ops, one board method per call: the
/// next op after `answers` (the ops answered so far), or `None` once the
/// task is created, claimed and its file reserved.
pub(super) fn worker_claim_op(answers: &[Value]) -> Option<Value> {
    match answers {
        [] => Some(
            json!({"op":"task_create","request":"w","title":"work","acceptance":["pass"],"dependencies":[]}),
        ),
        [task] => Some(json!({"op":"claim","task_id":task["id"]})),
        [task, claim] => Some(
            json!({"op":"reserve","task_id":task["id"],"token":claim["token"],"paths":["src/a.rs"]}),
        ),
        _ => None,
    }
}

/// `member` creates a task, claims it and reserves `src/a.rs`, one op per
/// board method; the claim's token.
pub(super) async fn worker_claim(workspace: &Path, member: &str) -> Value {
    let mut answers = Vec::new();
    while let Some(request) = worker_claim_op(&answers) {
        let answer = op(workspace, member, request.clone())
            .await
            .unwrap_or_else(|refusal| panic!("{member} {request}: {refusal}"));
        answers.push(answer);
    }
    answers[1]["token"].clone()
}

/// A harness-only board method (`_admit`, `_activate`) as `member`, through
/// the dispatcher over composition's handles: the call `SwarmContext` makes.
pub(super) fn board_call(workspace: &Path, member: &str, method: &str, args: Value) -> Value {
    let location = quecto::application::swarm::dto::BoardLocation {
        database: quecto::infrastructure::tools::swarm_bridge::store_database(workspace),
        checkout: workspace.to_path_buf(),
    };
    let handles = quecto::composition::swarm::build_swarm_board_handles(location, None);
    // A board call blocks: made off the async workers, as every board
    // caller is (#2278).
    quecto::infrastructure::tools::call_work::off_the_runtime(|| {
        quecto::infrastructure::tools::swarm_board_dispatch::call(&handles, member, method, args)
    })
    .unwrap_or_else(|refusal| panic!("{method} as {member}: {}", refusal.message()))
}

/// The coordinator admits and activates `member` as a live harness: this
/// process, by pid and start time, at `/tmp/<member>.sock`.
pub(super) fn admit_and_activate(workspace: &Path, member: &str, pid: u32, started: &str) {
    let reservation = format!("reservation-{member}");
    board_call(
        workspace,
        "coordinator",
        "_admit",
        json!([member, reservation]),
    );
    board_call(
        workspace,
        "coordinator",
        "_activate",
        json!([
            member,
            reservation,
            pid,
            started,
            format!("/tmp/{member}.sock")
        ]),
    );
}

/// One structured board op as `member` through the real swarm tool (#2281):
/// its answer, or the refusal's text.
pub(super) async fn op(workspace: &Path, member: &str, request: Value) -> Result<Value, String> {
    use quecto::application::tools::ports::Tool;
    let tool = quecto::infrastructure::tools::swarm::SwarmTool::new()
    .with_context(Some(context(workspace, member)));
    let result = tool
        .execute(&request.to_string())
        .await
        .map_err(|e| e.to_string())?;
    if result.is_error {
        return Err(result.content);
    }
    serde_json::from_str(&result.content).map_err(|e| format!("{e}: {}", result.content))
}

/// Consecutive contended reads a board poll sits out, and the pause after
/// each: the settlement watch's `SNAPSHOT_ATTEMPTS` and its one-second
/// pause (`swarm_lifecycle.rs`).
pub(super) const CONTENDED_READS: u32 = 10;
pub(super) const CONTENDED_READ_PAUSE: Duration = Duration::from_secs(1);

/// The store's refusal of a transaction that stayed busy past its 500 ms
/// timeout (#2278): its text exactly, as the tool boundary carries it.
/// Nothing else is contention.
pub(super) fn contended_read(error: &quecto::domain::error::DomainError) -> bool {
    matches!(
        error,
        quecto::domain::error::DomainError::Tool(text)
            if text == r#"swarm: "coordination store unavailable or contended: database is locked""#
    )
}
