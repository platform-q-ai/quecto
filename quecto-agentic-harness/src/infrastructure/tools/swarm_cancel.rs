//! Cancelling a background Python job by id (moved from swarm.rs).
use super::*;

pub(super) async fn cancel_op(
    v: &serde_json::Value,
    jobs: JobRegistry,
) -> Result<ToolResult, DomainError> {
    let id = job_id(v)?;
    let Some(job) = jobs.lock().unwrap().get(id).cloned() else {
        return ok_json(json!({"status":"not_found","job_id":id}), true);
    };
    let mut s = job.lock().unwrap();
    if s.status != "running" {
        return ok_json(
            json!({"status":s.status,"job_id":id,"execution_id":s.execution_id,"message":"job is already terminal"}),
            false,
        );
    }
    s.cancel_requested = true;
    if let Some(pid) = s.pid {
        kill_pid(pid);
        kill_pid_tree_best_effort(pid);
    }
    s.status = "cancelling".into();
    ok_json(
        json!({"status":"cancelling","job_id":id,"execution_id":s.execution_id}),
        false,
    )
}
