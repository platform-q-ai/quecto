use super::spawn::{SpawnTool, send_initial_prompt_to_socket};
use super::spawn_entry::{
    InitialRegistryEntrySpec, child_session_key, child_sidecar_filename, child_socket_path,
    effective_config_path, inherited_runtime_config_path, initial_registry_entry,
};
use super::spawn_launch_args::write_private_new;
use super::spawn_registry::register_and_broadcast;
use super::subagent_registry::{ExitSignal, new_exit_signal_channel};
use crate::domain::error::DomainError;
use crate::domain::ids::AgentUuid;
use crate::domain::subagent::SubagentConfig;
use crate::domain::subagent::{
    DisplayNameResolutionEntry, DisplayNameResolveError, assert_display_name_available_for_spawn,
};
use crate::domain::tool::ToolResult;
use crate::subagent_launch_app::{
    LaunchFuture, LaunchIdentity, PreparedRuntime, RegisteredLaunch,
    SubagentLaunchPorts as SubagentLaunchPortsTrait,
};
use std::path::{Path, PathBuf};

pub(super) struct SpawnLaunchPorts<'a> {
    tool: &'a SpawnTool,
    agent_uuid: Option<AgentUuid>,
    socket_path: Option<PathBuf>,
    owns_environment: bool,
}

impl<'a> SpawnLaunchPorts<'a> {
    pub(super) fn new(tool: &'a SpawnTool) -> Self {
        Self {
            tool,
            agent_uuid: None,
            socket_path: None,
            owns_environment: false,
        }
    }
}

impl<'a> SubagentLaunchPortsTrait for SpawnLaunchPorts<'a> {
    type Prepared = super::spawn_container::PreparedChild;

    fn allocate_identity(
        &mut self,
        config: &SubagentConfig,
    ) -> Result<LaunchIdentity, DomainError> {
        let session_name = config.agent_id.as_deref().unwrap_or("subagent").to_string();
        let entries = self.tool.registry.lock().unwrap_or_else(|e| e.into_inner());
        let resolution_entries: Vec<_> = entries
            .iter()
            .map(|(key, entry)| DisplayNameResolutionEntry {
                agent_uuid: entry.agent_uuid.clone(),
                display_name: entry.effective_display_name(key).to_string(),
                live: entry.status != super::subagent_registry::SubagentStatus::Exited,
            })
            .collect();
        if let Err(DisplayNameResolveError::AmbiguousLiveMatch { display_name })
        | Err(DisplayNameResolveError::NoLiveMatch { display_name }) =
            assert_display_name_available_for_spawn(&resolution_entries, &session_name)
        {
            return Err(DomainError::Tool(format!(
                "duplicate live subagent display label '{}'",
                display_name
            )));
        }
        drop(entries);
        let agent_uuid = AgentUuid::mint();
        let registry_key = agent_uuid.to_string();
        self.socket_path = Some(child_socket_path(&self.tool.socket_dir, &agent_uuid));
        self.agent_uuid = Some(agent_uuid);
        Ok(LaunchIdentity {
            session_name,
            registry_key,
        })
    }

    fn build_cli_args<'b>(
        &'b mut self,
        _identity: &'b LaunchIdentity,
        config: &'b SubagentConfig,
    ) -> Result<Vec<std::ffi::OsString>, DomainError> {
        let agent_uuid = self.agent_uuid.as_ref().expect("identity allocated");
        let socket_path = self.socket_path.as_ref().expect("socket path allocated");
        let workflow_spec_path = if let Some(ref spec) = config.workflow_spec {
            let spec_json = serde_json::to_string(spec).map_err(|e| {
                DomainError::Tool(format!("failed to serialize workflow spec: {e}"))
            })?;
            if spec_json.len() > crate::domain::workflow::MAX_WORKFLOW_SPEC_BYTES {
                return Err(DomainError::Tool(format!(
                    "workflow spec too large: {} bytes (max {})",
                    spec_json.len(),
                    crate::domain::workflow::MAX_WORKFLOW_SPEC_BYTES
                )));
            }
            let spec_path = self.tool.socket_dir.join(child_sidecar_filename(
                "quecto-wfspec",
                agent_uuid,
                std::process::id(),
            ));
            write_private_new(&spec_path, spec_json.as_bytes())
                .map_err(|e| DomainError::Tool(format!("failed to write workflow spec: {e}")))?;
            Some(spec_path)
        } else {
            None
        };
        let inherited_tool_policy =
            super::spawn_inherited_policy::snapshot(&self.tool.inherited_tool_policy);
        let inherited_tool_policy_path = if let Some(snapshot) = inherited_tool_policy.as_ref() {
            let path = self.tool.socket_dir.join(child_sidecar_filename(
                "quecto-tool-policy",
                agent_uuid,
                std::process::id(),
            ));
            super::inherited_tool_policy::write_snapshot(&path, snapshot).map_err(|e| {
                DomainError::Tool(format!("failed to write inherited tool policy: {e}"))
            })?;
            Some(path)
        } else {
            None
        };
        let effective_config =
            effective_config_path(config.config_path.as_ref(), inherited_runtime_config_path());
        Ok(super::spawn_launch_args::build_child_cli_args(
            &super::spawn_launch_args::ChildLaunchSpec {
                session_name: child_session_key(agent_uuid),
                socket_path,
                config,
                effective_config: effective_config.as_deref(),
                parent_id: self.tool.parent_id.as_deref(),
                restrict_to_workspace: self.tool.restrict_to_workspace,
                workflow_spec_path: workflow_spec_path.as_deref(),
                inherited_tool_policy_path: inherited_tool_policy_path.as_deref(),
            },
        ))
    }

    fn resolve_binary(&mut self) -> Result<PathBuf, DomainError> {
        super::spawn_binary::resolve_child_binary()
    }

    fn prepare_child<'b>(
        &'b mut self,
        config: &'b SubagentConfig,
        binary: &'b Path,
        cli_args: &'b [std::ffi::OsString],
    ) -> LaunchFuture<'b, Result<Self::Prepared, DomainError>> {
        Box::pin(async move {
            super::spawn_container::spawn_prepared_child(
                config,
                &super::spawn_container::ChildCommand {
                    binary,
                    cli_args,
                    base_dir: &self.tool.base_dir,
                },
                &self.tool.environment_registry,
            )
            .await
        })
    }

    fn ready<'b>(
        &'b mut self,
        prepared: &'b mut Self::Prepared,
    ) -> LaunchFuture<'b, Result<PreparedRuntime, DomainError>> {
        Box::pin(async move {
            self.owns_environment = prepared.owns_environment();
            let pid = prepared
                .child
                .as_ref()
                .and_then(|child| child.id())
                .unwrap_or(0);
            // #1369 slice 3: the endpoint the launch adapter prepared is
            // authoritative from here on. Only a LOCAL child (no endpoint)
            // uses the requested socket path; a proxy endpoint is
            // materialized into a parent-owned bridge socket and never falls
            // back to any requested direct path.
            let actual_socket_path = match prepared.endpoint.clone() {
                None => {
                    let socket_path = self
                        .socket_path
                        .as_ref()
                        .expect("socket path allocated")
                        .clone();
                    let child = prepared.child.as_mut().expect("local launch owns a child");
                    self.tool
                        .wait_for_socket_or_child_exit(&socket_path, child)
                        .await?;
                    socket_path
                }
                Some(crate::subagent_launch_app::ParentEndpoint::Direct { socket_path }) => {
                    self.tool.wait_for_socket(&socket_path).await?;
                    socket_path
                }
                Some(crate::subagent_launch_app::ParentEndpoint::Proxy { argv }) => {
                    let agent_key = self
                        .agent_uuid
                        .as_ref()
                        .expect("identity allocated")
                        .to_string();
                    let bridge = super::spawn_proxy_bridge::materialize(
                        argv,
                        &self.tool.socket_dir,
                        &agent_key,
                    )
                    .map_err(|e| {
                        DomainError::Tool(format!("failed to bind proxy bridge socket: {e}"))
                    })?;
                    let bridge_path = bridge.socket_path.clone();
                    prepared.proxy_bridge = Some(bridge);
                    super::spawn_proxy_bridge::wait_for_proxy_ready(&bridge_path).await?;
                    bridge_path
                }
            };
            Ok(PreparedRuntime {
                socket_path: actual_socket_path,
                pid,
                environment_ref: prepared.environment_ref.clone(),
            })
        })
    }

    fn rollback_prepared<'b>(
        &'b mut self,
        prepared: &'b mut Self::Prepared,
    ) -> LaunchFuture<'b, ()> {
        Box::pin(async move { prepared.rollback_once().await })
    }

    fn uncommit_registered<'b>(&'b mut self, registry_key: &'b str) -> LaunchFuture<'b, ()> {
        Box::pin(async move {
            // Abort the monitor inside the same critical section that removes
            // the entry, so a monitor cannot claim the cleanup plan after this
            // uncommit has decided to own it.
            let mut removed = {
                let mut entries = self.tool.registry.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(entry) = entries.get(registry_key) {
                    if let Some(ref handle) = entry.monitor_handle {
                        handle.abort();
                    }
                }
                entries
                    .remove(registry_key)
                    .map(|entry| vec![(registry_key.to_string(), entry)])
                    .unwrap_or_default()
            };
            for (_id, entry) in &removed {
                if let Some(ref tx) = entry.exit_signal_tx {
                    let _ = tx.send(Some(ExitSignal {
                        exit_code: None,
                        signal: Some(15),
                        kind: Default::default(),
                    }));
                }
                crate::infrastructure::tools::subagent_cascade::terminate_removed_entry(entry);
            }
            // Launch rollback: the environment's retained cleanup (not kill)
            // runs when the launch fails after creation (#1369 slice 2). A
            // creator's rollback also discards the record — the environment
            // never became usable, so it must not be listed as stopped.
            let mode = if self.owns_environment {
                super::subagent_cleanup::FinalizeMode::LaunchRollbackOwned
            } else {
                super::subagent_cleanup::FinalizeMode::LaunchRollback
            };
            super::subagent_cleanup::cleanup_removed_entries_once(&mut removed, mode).await;
        })
    }

    fn register_and_monitor<'b>(
        &'b mut self,
        identity: &'b LaunchIdentity,
        runtime: PreparedRuntime,
        prepared: &'b mut Self::Prepared,
        config: &'b SubagentConfig,
    ) -> LaunchFuture<'b, Result<RegisteredLaunch, DomainError>> {
        Box::pin(async move {
            let agent_uuid = self
                .agent_uuid
                .as_ref()
                .expect("identity allocated")
                .clone();
            let (cleanup_environment_id, cleanup_argv) = prepared.cleanup_plan();
            let (exit_tx, _exit_rx) = new_exit_signal_channel();
            let entry = initial_registry_entry(InitialRegistryEntrySpec {
                agent_uuid,
                display_name: identity.session_name.clone(),
                socket_path: runtime.socket_path.clone(),
                pid: runtime.pid,
                parent_id: self.tool.parent_id.clone(),
                config,
                exit_signal_tx: Some(exit_tx.clone()),
                cleanup_environment_id,
                cleanup_argv,
                environment_registry: prepared
                    .environment_ref
                    .as_ref()
                    .map(|_| self.tool.environment_registry.clone()),
                environment_ref: prepared.environment_ref.clone(),
            });
            register_and_broadcast(
                &self.tool.registry,
                self.tool.broadcast_tx.as_ref(),
                &identity.session_name,
                entry,
            )?;
            // Slice 2 (#1369): every registered environment child — creator or
            // joiner — becomes a member of its environment; the registry key is
            // the agent UUID. add_member is refused once the environment is no
            // longer running (a join racing a kill), in which case the launch
            // fails: unregister the entry we just added and let the use case
            // roll back.
            if let Some(env_ref) = prepared.environment_ref.as_deref() {
                if let Err(e) = self
                    .tool
                    .environment_registry
                    .add_member(env_ref, &identity.registry_key)
                {
                    // Unregister AND re-broadcast the survivor set: the entry
                    // was announced by register_and_broadcast, so clients must
                    // see it withdrawn, not linger as a phantom agent.
                    let event = {
                        let mut entries =
                            self.tool.registry.lock().unwrap_or_else(|e| e.into_inner());
                        entries.remove(&identity.registry_key);
                        self.tool.broadcast_tx.as_ref().map(|_| {
                            crate::infrastructure::tools::subagent_cascade::build_state_changed_event_locked(&entries)
                        })
                    };
                    if let (Some(tx), Some(event)) = (self.tool.broadcast_tx.as_ref(), event) {
                        let _ = tx.send(event);
                    }
                    return Err(DomainError::Tool(format!(
                        "cannot register into environment {env_ref}: {e}"
                    )));
                }
            }
            let monitor_handle = super::subagent_monitor::spawn_monitor_task(
                identity.registry_key.clone(),
                runtime.socket_path.clone(),
                self.tool.registry.clone(),
                self.tool.notify_tx.clone(),
                self.tool.broadcast_tx.clone(),
                self.tool.parent_id.clone(),
            );
            let proxy_bridge = prepared.proxy_bridge.take().map(|bridge| {
                let (socket, handle) = bridge.into_parts();
                (socket, std::sync::Arc::new(handle))
            });
            {
                let mut entries = self.tool.registry.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(entry) = entries.get_mut(&identity.registry_key) {
                    entry.monitor_handle = Some(std::sync::Arc::new(monitor_handle));
                    if let Some((socket, handle)) = proxy_bridge {
                        entry.proxy_bridge_socket = Some(socket);
                        entry.proxy_bridge_handle = Some(handle);
                    }
                }
            }
            if let Some(child) = prepared.child.take() {
                super::spawn_reaper::spawn_reaper_task(
                    child,
                    self.tool.registry.clone(),
                    identity.registry_key.clone(),
                    exit_tx,
                    self.tool.broadcast_tx.clone(),
                );
            }
            Ok(RegisteredLaunch {
                registry_key: identity.registry_key.clone(),
                socket_path: runtime.socket_path,
            })
        })
    }

    fn send_initial_prompt<'b>(
        &'b mut self,
        socket_path: &'b Path,
        task: &'b str,
    ) -> LaunchFuture<'b, Result<(), DomainError>> {
        Box::pin(async move { send_initial_prompt_to_socket(socket_path, task).await })
    }
    fn success(&self, identity: &LaunchIdentity, environment_ref: Option<&str>) -> ToolResult {
        let env_ref = environment_ref
            .map(|r| {
                // Members of one environment share its reported workspace, so
                // the spawn result names it alongside the ref (#1369 slice 2).
                let workspace = self
                    .tool
                    .environment_registry
                    .get(r)
                    .map(|record| format!(" workspace={}", record.workspace_path.display()))
                    .unwrap_or_default();
                format!(" environment_ref={r}{workspace}")
            })
            .unwrap_or_default();
        ToolResult {
            content: format!(
                "Subagent '{}' is running (uuid={}){}. Use agent_cmd to interact.",
                identity.session_name, identity.registry_key, env_ref
            ),
            is_error: false,
            image_blocks: vec![],
        }
    }
}

#[cfg(test)]
#[path = "spawn_launch_ports_cov_tests.rs"]
mod cov_tests;
