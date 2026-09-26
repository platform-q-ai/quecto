//! What dropping a launch part-way undoes (#2173 review): a cancelled
//! launch, before registration or before its initial prompt arrived.
use super::SpawnLaunchPorts;

/// How far a launch got (#2173 review).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum LaunchStage {
    /// Not registered: the child (if any) never bound, so it ends itself
    /// at its bind deadline; its sidecars and admission scope are ours.
    Unregistered,
    /// Registered under this key, its initial prompt not yet delivered.
    AwaitingPrompt(String),
    /// Delivered, or compensated: nothing is left to undo.
    Complete,
}

/// A launch dropped part-way was cancelled (#2173 review). Before
/// registration its capability files go and its admission scope is
/// retired, as on any failure before the child started; registered but
/// never given its task, it is compensated as a failed launch is.
impl Drop for SpawnLaunchPorts<'_> {
    fn drop(&mut self) {
        match std::mem::replace(&mut self.stage, LaunchStage::Complete) {
            LaunchStage::Unregistered => self.abandon_unregistered(),
            LaunchStage::AwaitingPrompt(registry_key) => self.compensate_detached(registry_key),
            LaunchStage::Complete => {}
        }
    }
}

impl SpawnLaunchPorts<'_> {
    /// Compensate a registered launch whose initial prompt never arrived,
    /// in a counted task (the ports borrow the tool; the use cases don't).
    fn compensate_detached(&mut self, registry_key: String) {
        let child = {
            let entries = self.tool.registry.lock().unwrap_or_else(|e| e.into_inner());
            entries
                .get(&registry_key)
                .and_then(super::super::subagent_registry::SubagentEntry::delegated_identity)
        };
        let (Some(child), Ok(lifecycle), Ok(runtime)) = (
            child,
            self.tool.lifecycle_use_cases(),
            tokio::runtime::Handle::try_current(),
        ) else {
            tracing::warn!(agent = %registry_key, "a cancelled launch could not be compensated");
            return;
        };
        let registry = self.tool.registry.clone();
        let owns_environment = self.owns_environment;
        let flight = super::super::launch_rollbacks::InFlight::enter();
        runtime.spawn(async move {
            lifecycle
                .compensate_launch
                .execute(
                    crate::application::subagents::dto::CompensateFailedLaunchRequest {
                        child,
                        owns_environment,
                    },
                )
                .await;
            registry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&registry_key);
            drop(flight);
        });
    }

    /// Remove what an unregistered launch leaves on disk and retire its
    /// admission scope in a counted task a stopping run waits for.
    fn abandon_unregistered(&mut self) {
        self.discard_unconsumed_parent_control_sidecar();
        let Some((path, credential)) = self.admission_registration.take() else {
            return;
        };
        let _ = std::fs::remove_file(&path);
        let (Some(admission), Ok(runtime)) = (
            crate::infrastructure::admission::process::current(),
            tokio::runtime::Handle::try_current(),
        ) else {
            tracing::warn!("an unregistered child's admission scope could not be retired");
            return;
        };
        let flight = super::super::launch_rollbacks::InFlight::enter();
        runtime.spawn(async move {
            if let Err(error) = admission.connection().retire_child(credential.scope).await {
                tracing::warn!(%error, "failed to retire the unlaunched child's admission scope");
            }
            drop(flight);
        });
    }
}
