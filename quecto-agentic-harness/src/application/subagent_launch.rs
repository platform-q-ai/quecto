use std::ffi::OsString;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use crate::domain::error::DomainError;
use crate::domain::subagent::SubagentConfig;
use crate::domain::tool::ToolResult;

/// Pure application orchestration for a subagent launch transaction.
///
/// Infrastructure owns every side effect behind [`SubagentLaunchPorts`]; this
/// use case owns the order and rollback invariant: prepare, wait for readiness,
/// register/monitor, optionally send the initial prompt, and clean up exactly
/// once on each failing phase.
pub struct SubagentLaunchUseCase<P> {
    ports: P,
}

impl<P> SubagentLaunchUseCase<P> {
    pub fn new(ports: P) -> Self {
        Self { ports }
    }
}

pub type LaunchFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait SubagentLaunchPorts {
    type Prepared;

    fn allocate_identity(&mut self, config: &SubagentConfig)
    -> Result<LaunchIdentity, DomainError>;
    fn build_cli_args<'a>(
        &'a mut self,
        identity: &'a LaunchIdentity,
        config: &'a SubagentConfig,
    ) -> Result<Vec<OsString>, DomainError>;
    fn resolve_binary(&mut self) -> Result<PathBuf, DomainError>;
    fn prepare_child<'a>(
        &'a mut self,
        config: &'a SubagentConfig,
        binary: &'a Path,
        cli_args: &'a [OsString],
    ) -> LaunchFuture<'a, Result<Self::Prepared, DomainError>>;
    fn ready<'a>(
        &'a mut self,
        prepared: &'a mut Self::Prepared,
    ) -> LaunchFuture<'a, Result<PreparedRuntime, DomainError>>;
    fn rollback_prepared<'a>(
        &'a mut self,
        prepared: &'a mut Self::Prepared,
    ) -> LaunchFuture<'a, ()>;
    /// Atomically uncommit a registered launch: take/remove the registry record,
    /// cancel its monitor/ownership, terminate the runtime if appropriate, and
    /// run any claimed cleanup exactly once. This prevents prompt-failure races
    /// with monitor-owned cleanup.
    fn uncommit_registered<'a>(&'a mut self, registry_key: &'a str) -> LaunchFuture<'a, ()>;
    fn register_and_monitor<'a>(
        &'a mut self,
        identity: &'a LaunchIdentity,
        runtime: PreparedRuntime,
        prepared: &'a mut Self::Prepared,
        config: &'a SubagentConfig,
    ) -> LaunchFuture<'a, Result<RegisteredLaunch, DomainError>>;
    fn send_initial_prompt<'a>(
        &'a mut self,
        socket_path: &'a Path,
        task: &'a str,
    ) -> LaunchFuture<'a, Result<(), DomainError>>;
    fn success(&self, identity: &LaunchIdentity, environment_ref: Option<&str>) -> ToolResult;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchIdentity {
    pub session_name: String,
    pub registry_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRuntime {
    pub socket_path: PathBuf,
    pub pid: u32,
    pub environment_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredLaunch {
    pub registry_key: String,
    pub socket_path: PathBuf,
}

impl<P> SubagentLaunchUseCase<P>
where
    P: SubagentLaunchPorts + Send,
    P::Prepared: Send,
{
    pub async fn execute(mut self, config: &SubagentConfig) -> Result<ToolResult, DomainError> {
        let identity = self.ports.allocate_identity(config)?;
        let cli_args = self.ports.build_cli_args(&identity, config)?;
        let binary = self.ports.resolve_binary()?;
        let mut prepared = self.ports.prepare_child(config, &binary, &cli_args).await?;

        let runtime = match self.ports.ready(&mut prepared).await {
            Ok(runtime) => runtime,
            Err(e) => {
                self.ports.rollback_prepared(&mut prepared).await;
                return Err(e);
            }
        };
        let environment_ref = runtime.environment_ref.clone();

        let registered = match self
            .ports
            .register_and_monitor(&identity, runtime, &mut prepared, config)
            .await
        {
            Ok(registered) => registered,
            Err(e) => {
                self.ports.rollback_prepared(&mut prepared).await;
                return Err(e);
            }
        };

        if let Some(task) = config.task.as_deref() {
            if let Err(e) = self
                .ports
                .send_initial_prompt(&registered.socket_path, task)
                .await
            {
                self.ports
                    .uncommit_registered(&registered.registry_key)
                    .await;
                return Err(e);
            }
        }

        Ok(self.ports.success(&identity, environment_ref.as_deref()))
    }
}
