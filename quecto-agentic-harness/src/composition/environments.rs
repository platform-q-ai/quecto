//! The concrete graph of the environments capability (#1369, #1939): the
//! inventory query and the environment kill `agent_cmd` serves, over the
//! session's environment registry, the script command adapters and the
//! member shutdown composition binds; and the final-member cleanup a
//! compensation runs for each membership it removes. Built once per
//! harness beside its agent-control tools and installed in the slots the
//! built tools read. And the container-runtime doctor (#2024 S4b): the
//! create preflight of the effective container config, resolved through
//! the same launch-policy selection a spawn uses. And the durable registry
//! (#2024 S4d): every session's registry is restored from — and journals
//! through — the base directory's `environments.json`, and the CLI's
//! `container ls|kill|gc` run over the same restore.
use std::path::Path;
use std::sync::Arc;

use crate::application::configuration::dto::ConfigSelection;
use crate::application::environments::dto::{RestoreMode, RestoredRegistry};
use crate::application::environments::ports::{
    ContainerConfigLookup, ContainerRuntimeInventory, ContainerRuntimePreflight,
    EnvironmentMemberShutdown, EnvironmentProcess, EnvironmentRegistryStore, MemberShutdownReport,
    PortFuture, UnsettledMember,
};
use crate::application::environments::use_cases::{
    DiagnoseContainerRuntime, FinalizeEnvironmentMember, GcOrphanedEnvironments, KillEnvironment,
    ListEnvironmentsQuery, RestoreRegistry,
};
use crate::domain::environment_registry::EnvironmentRegistry;
use crate::infrastructure::config::container_config_lookup::SelectedConfigLookup;
use crate::infrastructure::persistence::environment_registry_store::FileEnvironmentRegistryStore;
use crate::infrastructure::processes::containers::environment_process::ScriptEnvironmentProcess;
use crate::infrastructure::processes::containers::preflight::ScriptPreflight;
use crate::infrastructure::processes::containers::script_inventory::ScriptInventory;
pub use crate::infrastructure::tools::agent_cmd_containers::EnvironmentControl;
use crate::infrastructure::tools::environment_commands::{
    HostedStoreObservation, ScriptEnvironmentCommands,
};
pub use crate::interface::cli::container_handles::ContainerInventoryHandles;

/// The final-member use case over the production script adapters, for one
/// entry's environment registry. The cleanup jobs already run on a blocking
/// worker, so the scripts run inline.
pub fn build_member_finalizer(environments: EnvironmentRegistry) -> FinalizeEnvironmentMember {
    FinalizeEnvironmentMember::new(
        environments,
        Arc::new(ScriptEnvironmentCommands::inline()),
        Arc::new(HostedStoreObservation),
    )
}

/// The environment control `agent_cmd` invokes: the side-effect-free
/// listing and the kill that asks the environment's members to shut down
/// through `member_shutdown` before its retained kill.
pub fn build_environment_control(
    environments: EnvironmentRegistry,
    member_shutdown: Arc<dyn EnvironmentMemberShutdown>,
) -> EnvironmentControl {
    EnvironmentControl {
        list: Arc::new(ListEnvironmentsQuery::new(environments.clone())),
        kill: Arc::new(KillEnvironment::new(
            environments,
            member_shutdown,
            Arc::new(ScriptEnvironmentCommands::default()),
        )),
    }
}

/// The doctor `quecto container doctor` invokes (#2024 S4b), over the
/// run's own configuration selection — the layers resolved for its
/// working directory, or the explicit `--config` file — with trust
/// recorded under `base_dir`. `main` hands this builder to the CLI entry
/// point; the interface never composes the selection or runs a script.
pub fn build_container_doctor(
    base_dir: &Path,
    selection: &ConfigSelection,
) -> Arc<DiagnoseContainerRuntime> {
    Arc::new(DiagnoseContainerRuntime::new(
        build_container_config_lookup(base_dir, Some(selection.clone())),
        build_container_runtime_preflight(),
    ))
}

/// The lookup port adapter alone, for the contract suite: the doctor's
/// target resolved through the launch policy's selection.
pub fn build_container_config_lookup(
    base_dir: &Path,
    selection: Option<ConfigSelection>,
) -> Arc<dyn ContainerConfigLookup> {
    Arc::new(SelectedConfigLookup::new(
        super::container_configs::build_container_config_selection(base_dir, selection),
    ))
}

/// The preflight port adapter alone, for the contract suite.
pub fn build_container_runtime_preflight() -> Arc<dyn ContainerRuntimePreflight> {
    Arc::new(ScriptPreflight)
}

// ─── Durable registry (#2024 S4d) ────────────────────────────────────────────

/// The durable registry store of `base_dir`, for contracts and the
/// builders below.
pub fn build_environment_registry_store(base_dir: &Path) -> Arc<dyn EnvironmentRegistryStore> {
    Arc::new(FileEnvironmentRegistryStore::for_base_dir(base_dir))
}

/// The liveness/cleanup adapter over a record's retained scripts.
pub fn build_environment_process() -> Arc<dyn EnvironmentProcess> {
    Arc::new(ScriptEnvironmentProcess)
}

/// The host's environment inventory through a config's own scripts.
pub fn build_container_runtime_inventory() -> Arc<dyn ContainerRuntimeInventory> {
    Arc::new(ScriptInventory)
}

/// The restore use case over the production adapters for `base_dir`.
pub fn build_restore_registry(base_dir: &Path) -> RestoreRegistry {
    RestoreRegistry::new(
        build_environment_registry_store(base_dir),
        build_environment_process(),
        Arc::new(HostedStoreObservation),
    )
}

/// The session's environment registry (#2024 S4d): durable through
/// `<base_dir>/environments.json` — refs allocated there, every
/// transition journalled — and, for a top-level session, seeded with the
/// records earlier (or concurrent) sessions wrote, each checked against
/// the runtime. A spawned child journals its own creates but is not
/// seeded: what it sees of the fleet is its parent's to show. `main`
/// hands this builder to the CLI; the registry itself is built exactly
/// once per harness, before the tools that commit to it.
pub fn build_environment_registry(
    base_dir: &Path,
    session: &str,
    seed: bool,
) -> crate::interface::cli::EnvironmentRegistryBuild {
    use crate::interface::cli::EnvironmentRegistryBuild;

    let restore = build_restore_registry(base_dir);
    if !seed {
        return EnvironmentRegistryBuild {
            registry: restore.unseeded(session),
            reconciliation: None,
        };
    }
    let prepared = restore.prepare(session);
    report_restore(&prepared.report);
    EnvironmentRegistryBuild {
        registry: prepared.registry,
        reconciliation: prepared.reconciliation,
    }
}

/// Present the result of an asynchronous agent-startup reconciliation.
/// Kept in composition so the interface owns neither logging policy nor
/// environment-domain report formatting.
pub fn report_environment_reconciliation(report: &RestoredRegistry) {
    report_restore(report);
}

fn report_restore(report: &RestoredRegistry) {
    if let Some(read_error) = &report.read_error {
        // The session starts with an empty registry and every container
        // create refused (the store fails to allocate the same way).
        eprintln!("{read_error}; container creates are refused until it is repaired");
    }
    for line in &report.diagnostics {
        eprintln!("{line}");
    }
    if !report.restored.is_empty()
        || !report.stopped.is_empty()
        || !report.retained.is_empty()
        || !report.unverified.is_empty()
    {
        tracing::info!(
            restored = report.restored.len(),
            stopped = report.stopped.len(),
            retained = report.retained.len(),
            unverified = report.unverified.len(),
            "environment registry restored"
        );
    }
    for (environment_ref, reason) in &report.retained {
        eprintln!("{environment_ref} retained at restore: {reason}");
    }
    for (environment_ref, reason) in &report.unverified {
        tracing::warn!(environment_ref, %reason, "restored environment could not be verified against the runtime");
    }
}

/// The environment handles `quecto container ls|kill|gc` invoke (#2024
/// S4d): the listing, the kill and the collector over a registry restored
/// from `base_dir` for the command's own run (no members of any session
/// are reachable from here, so a kill settles none and runs the retained
/// kill directly). The collector lists and removes unrecorded orphans
/// through the container config the run's own selection resolves, like
/// the doctor. An observing `mode` (a `gc --dry-run`) restores without
/// writing a correction (round 3 H1, #2033).
pub fn build_container_inventory(
    base_dir: &Path,
    selection: &ConfigSelection,
    mode: RestoreMode,
) -> ContainerInventoryHandles {
    // The restore's account (diagnostics, a read error) is the handles'
    // to carry: the command's presenter reports it and refuses on the
    // read error; composition prints nothing.
    let restore = build_restore_registry(base_dir);
    let (registry, restore) = match mode {
        RestoreMode::Correct => restore.execute("cli"),
        RestoreMode::Observe => restore.observe("cli"),
    };
    ContainerInventoryHandles {
        list: Arc::new(ListEnvironmentsQuery::new(registry.clone())),
        kill: Arc::new(KillEnvironment::new(
            registry.clone(),
            Arc::new(NoReachableMembers),
            Arc::new(ScriptEnvironmentCommands::default()),
        )),
        gc: Arc::new(GcOrphanedEnvironments::new(
            registry,
            build_container_config_lookup(base_dir, Some(selection.clone())),
            build_container_runtime_inventory(),
            build_environment_process(),
            Arc::new(HostedStoreObservation),
        )),
        restore,
    }
}

/// The CLI holds no session: a member recorded on an environment (none
/// on a restored record — members are never stored) cannot be asked to
/// shut down from here, so it is reported unsettled rather than
/// pretended gone. Pure adaptation, no effect: composition's to define.
struct NoReachableMembers;

impl EnvironmentMemberShutdown for NoReachableMembers {
    fn shutdown_members<'a>(
        &'a self,
        members: &'a [String],
    ) -> PortFuture<'a, MemberShutdownReport> {
        Box::pin(async move {
            MemberShutdownReport {
                settled: Vec::new(),
                unsettled: members
                    .iter()
                    .map(|member| UnsettledMember {
                        member: member.clone(),
                        detail:
                            "no session holds this member; kill it from its session or let it exit"
                                .to_string(),
                    })
                    .collect(),
            }
        })
    }
}
