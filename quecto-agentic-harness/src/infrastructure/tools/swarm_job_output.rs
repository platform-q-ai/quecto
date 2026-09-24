//! Reading a background Python job's output by id (moved from swarm.rs).
use super::*;

pub(super) async fn output_op(
    v: &serde_json::Value,
    workspace: Arc<PathBuf>,
    jobs: JobRegistry,
) -> Result<ToolResult, DomainError> {
    let id = job_id(v)?;
    let offset = bounded_u64(v, "offset", 0, u64::MAX).map_err(DomainError::Other)? as usize;
    let limit = bounded_u64(v, "limit", 200_000, 1_000_000).map_err(DomainError::Other)? as usize;
    let Some(job) = jobs.lock().unwrap().get(id).cloned() else {
        return ok_json(json!({"status":"not_found","job_id":id}), true);
    };
    let (status, exit_code, outp, errp, result) = {
        let s = job.lock().unwrap();
        (
            s.status.clone(),
            s.exit_code,
            s.stdout_path.clone(),
            s.stderr_path.clone(),
            s.result.clone(),
        )
    };
    let stdout = read_slice(&outp, offset, limit).await?;
    let stderr = read_slice(&errp, offset, limit).await?;
    // Paging reads the artifacts back off disk so callers can walk output far
    // larger than the inline preview. Nothing stops a later program from
    // rewriting those files, so the sizes captured at completion are compared
    // against what is on disk now and any divergence is surfaced.
    let artifacts_modified = artifacts_diverged(result.as_ref(), &outp, &errp).await;
    let is_err = (status != "running" && status != "cancelling" && status != "completed")
        || (status == "completed" && exit_code.unwrap_or(0) != 0);
    ok_json(
        json!({"status":status,"job_id":id,"stdout":stdout.0,"stderr":stderr.0,"offset":offset,"limit":limit,"stdout_more":stdout.1,"stderr_more":stderr.1,"result":result,"artifacts_modified":artifacts_modified,"artifact_namespace":"workspace-relative","artifact_base":workspace.as_ref(),"artifact_paths":[rel(&workspace,&outp),rel(&workspace,&errp)]}),
        is_err,
    )
}
