//! The container create a spawn runs detached (#2173): the create script,
//! then the record of what it made. A spawn cancelled meanwhile stops it.
use super::*;

/// What a detached container create needs, owned.
pub(super) struct CreateJob {
    /// Released unless the create commits, even if the job never runs.
    pub(super) minted: MintedRef,
    pub(super) selected: SelectedContainerConfig,
    pub(super) name: Option<String>,
    pub(super) admission_dir: Option<std::path::PathBuf>,
    pub(super) supervisor: Arc<OwnedChildSupervisor>,
}

/// Run the create script, then commit the environment it made.
pub(super) async fn create_environment(
    cmd: tokio::process::Command,
    job: CreateJob,
    abandoned: impl std::future::Future<Output = ()>,
) -> Result<PreparedChild, DomainError> {
    let CreateJob {
        minted,
        selected,
        name,
        admission_dir,
        supervisor,
    } = job;
    let container = &selected.config;
    let config_name = container.name.as_str();
    // Every return before the commit gives the minted ref back.
    let output = match run_stoppable_script(cmd, "create", abandoned).await? {
        Some(output) => output,
        None => {
            return Err(DomainError::Tool(
                "container create stopped: its spawn was cancelled".into(),
            ));
        }
    };
    let result = match parse_create_result(&output.stdout, admission_dir.as_deref()) {
        Ok(result) => result,
        Err(e) => {
            let mut cleanup_argv = container.cleanup.clone();
            if let Some(env_id) = salvage_environment_id(&output.stdout) {
                run_cleanup_once(Some(env_id), &mut cleanup_argv).await;
            }
            return Err(e);
        }
    };
    if result
        .metadata
        .get(super::super::environment_commands::CHECKOUT_METADATA_KEY)
        .and_then(serde_json::Value::as_str)
        .is_none_or(str::is_empty)
    {
        // Without it the host probes <workspace>/repo and <workspace> for a
        // coordination store before deciding whether a swarm's box must be
        // kept (#1924); say so once per create so a script author notices.
        tracing::warn!(
            environment_id = %result.environment_id,
            script = %config_name,
            "create result omits metadata.checkout; swarm retention will probe the workspace for the coordination store"
        );
    }
    let environment_ref = minted.as_str().to_string();
    let environments = minted.registry().clone();
    minted.commit(EnvironmentRecord {
        environment_ref: environment_ref.clone(),
        environment_id: result.environment_id.clone(),
        environment_uuid:
            crate::domain::environments::entities::environment_registry::mint_environment_uuid(),
        name,
        workspace_path: result.workspace_path.clone(),
        // The config owns its source (#1410): the repository shown in
        // listings/TUI is whatever the create script truthfully reported in
        // its metadata; sandbox configs report none.
        repository: reported_repository(&result.metadata),
        script_name: config_name.to_string(),
        retained_exec_argv: container.exec.clone(),
        retained_kill_argv: container.kill.clone(),
        retained_cleanup_argv: container.cleanup.clone(),
        retained_inspect_argv: container.inspect.clone(),
        members: Vec::new(),
        status:
            crate::domain::environments::entities::environment_registry::EnvironmentStatus::Running,
        metadata: result.metadata.clone(),
        last_error: None,
        origin:
            crate::domain::environments::entities::environment_registry::EnvironmentOrigin::Created,
        created_by: environments.session().to_string(),
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|since| since.as_secs()),
    });
    Ok(PreparedChild {
        swarm_reservation: None,
        owned_child: None,
        display_pid: 0,
        supervisor,
        environment_ref: Some(environment_ref),
        endpoint: Some(result.endpoint),
        proxy_bridge: None,
        process_owner: super::super::process_tree::ProcessOwner::DirectPid,
        cleanup_environment_id: Some(result.environment_id),
        cleanup_argv: container.cleanup.clone(),
        environments: Some(environments.clone()),
        stderr_tail: None,
        container_diagnostics: selected.diagnostics,
        settled: false,
    })
}

/// How long a stopped create is waited for while it removes what it made;
/// past it the script finishes on its own. Inside the exit's 5 s wait.
const CREATE_STOP_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

/// [`run_script`] that `stop` can end: `None` once stopped.
async fn run_stoppable_script(
    cmd: tokio::process::Command,
    operation: &str,
    stop: impl std::future::Future<Output = ()>,
) -> Result<Option<ScriptOutput>, DomainError> {
    use crate::infrastructure::processes::containers::script_stderr::{
        ScriptRun, run_stoppable_capturing_stderr_tail,
    };
    let run =
        run_stoppable_capturing_stderr_tail(cmd, ScriptStdout::Result, stop, CREATE_STOP_GRACE)
            .await
            .map_err(|e| {
                DomainError::Tool(format!("failed to invoke script-managed {operation}: {e}"))
            })?;
    let output = match run {
        ScriptRun::Finished(output) => output,
        ScriptRun::Stopped => return Ok(None),
    };
    match output.status.success() {
        true => Ok(Some(output)),
        false => {
            let message = output.failure_message(operation);
            eprintln!("{message}");
            Err(DomainError::Tool(message))
        }
    }
}
