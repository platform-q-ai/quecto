use crate::domain::error::DomainError;
use crate::domain::subagent::SubagentConfig;
use crate::domain::tool::ToolResult;

pub use crate::domain::subagent_launch::{
    LaunchFuture, LaunchIdentity, ParentEndpoint, PreparedRuntime, RegisteredLaunch,
    SubagentLaunchPorts,
};

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
            let retry_until = self.ports.initial_prompt_retry_deadline();
            loop {
                match self
                    .ports
                    .send_initial_prompt(&registered.socket_path, task)
                    .await
                {
                    Ok(()) => break,
                    Err(e) => {
                        let Some(deadline) = retry_until else {
                            self.ports
                                .uncommit_registered(&registered.registry_key)
                                .await;
                            return Err(e);
                        };
                        if tokio::time::Instant::now() >= deadline {
                            self.ports
                                .uncommit_registered(&registered.registry_key)
                                .await;
                            return Err(e);
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                }
            }
        }

        Ok(self.ports.success(&identity, environment_ref.as_deref()))
    }
}
