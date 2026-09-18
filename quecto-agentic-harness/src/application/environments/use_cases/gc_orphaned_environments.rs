//! Collect orphaned environments (#2024 S4d): `quecto container gc`.
//!
//! An orphan is an environment nothing will ever address again: its
//! runtime container is gone or exited AND the registry either knows
//! nothing about it or records it `stopped`. Everything else is kept and
//! said so — a `running`, `retained`, `killing` or `cleanup-failed` record
//! with a live container, an unknown state dir whose container still runs,
//! or a record whose liveness the runtime could not confirm. The collector
//! never kills: a stopped record is removed through its own retained
//! `cleanup` (the script that knows the layout), an unrecorded orphan by
//! removing its exited container and then its state dir. A dry run reports
//! the same judgement without an effect.
//!
//! The state roots scanned are the ones the request names plus the ones
//! the registry's records imply (the parent of each record's environment
//! directory); containers the runtime lists under the environment label
//! are judged even when their state dir is already gone.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};

use super::super::dto::{
    EnvironmentLiveness, GcCandidate, GcKept, GcRemoval, GcReport, GcRequest, RuntimeContainer,
};
use super::super::ports::{ContainerRuntimeInventory, EnvironmentProcess};

pub struct GcOrphanedEnvironments {
    registry: EnvironmentRegistry,
    inventory: Arc<dyn ContainerRuntimeInventory>,
    process: Arc<dyn EnvironmentProcess>,
}

impl std::fmt::Debug for GcOrphanedEnvironments {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GcOrphanedEnvironments")
            .field("registry", &self.registry)
            .finish_non_exhaustive()
    }
}

/// The state root a record's workspace implies: the parent of the
/// environment directory, which is the ancestor named after the
/// environment id (`<root>/<environment_id>/workspace[/repo]`).
pub fn implied_state_root(record: &EnvironmentRecord) -> Option<PathBuf> {
    record
        .workspace_path
        .ancestors()
        .find(|ancestor| {
            ancestor
                .file_name()
                .is_some_and(|name| name == record.environment_id.as_str())
        })
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

/// Everything known about one environment id before it is judged.
struct Sighting<'a> {
    environment_id: &'a str,
    state_dir: Option<&'a Path>,
    /// The container the state dir names (or the runtime listed).
    container_name: Option<&'a str>,
    /// The runtime's own entry for it, when it lists one.
    container: Option<&'a RuntimeContainer>,
    record: Option<&'a EnvironmentRecord>,
}

impl GcOrphanedEnvironments {
    pub fn new(
        registry: EnvironmentRegistry,
        inventory: Arc<dyn ContainerRuntimeInventory>,
        process: Arc<dyn EnvironmentProcess>,
    ) -> Self {
        Self {
            registry,
            inventory,
            process,
        }
    }

    pub fn execute(&self, request: &GcRequest) -> GcReport {
        let mut report = GcReport {
            dry_run: request.dry_run,
            ..GcReport::default()
        };
        let records = self.registry.entries();
        let by_id: BTreeMap<&str, &EnvironmentRecord> = records
            .iter()
            .map(|record| (record.environment_id.as_str(), record))
            .collect();

        let mut roots: BTreeSet<PathBuf> = request.state_roots.iter().cloned().collect();
        roots.extend(records.iter().filter_map(implied_state_root));
        report.state_roots = roots.iter().cloned().collect();

        let containers = match self.inventory.containers() {
            Ok(containers) => containers,
            Err(error) => {
                // Without the runtime's word nothing can be called exited:
                // judge nothing, say why.
                report
                    .errors
                    .push(format!("container runtime inventory unavailable: {error}"));
                return report;
            }
        };
        let container_by_name: BTreeMap<&str, &RuntimeContainer> = containers
            .iter()
            .map(|container| (container.name.as_str(), container))
            .collect();

        let mut judged: BTreeSet<String> = BTreeSet::new();
        for root in &roots {
            let dirs = match self.inventory.environment_dirs(root) {
                Ok(dirs) => dirs,
                Err(error) => {
                    report.errors.push(format!(
                        "state root {} not scanned: {error}",
                        root.display()
                    ));
                    continue;
                }
            };
            for dir in dirs {
                judged.insert(dir.environment_id.clone());
                let container = dir
                    .container
                    .as_deref()
                    .and_then(|name| container_by_name.get(name).copied());
                self.judge(
                    Sighting {
                        environment_id: &dir.environment_id,
                        state_dir: Some(&dir.path),
                        container_name: dir.container.as_deref(),
                        container,
                        record: by_id.get(dir.environment_id.as_str()).copied(),
                    },
                    &mut report,
                );
            }
        }
        // Containers whose state dir is already gone (or lives under a
        // root nobody named).
        for container in &containers {
            let Some(environment_id) = container.environment_id.as_deref() else {
                continue;
            };
            if !judged.insert(environment_id.to_string()) {
                continue;
            }
            self.judge(
                Sighting {
                    environment_id,
                    state_dir: None,
                    container_name: Some(container.name.as_str()),
                    container: Some(container),
                    record: by_id.get(environment_id).copied(),
                },
                &mut report,
            );
        }
        if !request.dry_run {
            self.remove(&mut report);
        }
        report
    }

    /// One environment's verdict from its state dir, container and record.
    fn judge(&self, sighting: Sighting<'_>, report: &mut GcReport) {
        let Sighting {
            environment_id,
            state_dir,
            container_name,
            container,
            record,
        } = sighting;
        let keep = |report: &mut GcReport, reason: String| {
            report.kept.push(GcKept {
                environment_id: environment_id.to_string(),
                reason,
            })
        };
        if let Some(container) = container.filter(|container| container.running) {
            keep(report, format!("container {} is running", container.name));
            return;
        }
        let container_state = match (container_name, container) {
            (Some(name), Some(_)) => format!("container {name} exited"),
            (Some(name), None) => format!("container {name} not known to the runtime"),
            (None, _) => "no container recorded".to_string(),
        };
        let removal = match record {
            None => GcRemoval::Direct,
            Some(record) => match record.status {
                EnvironmentStatus::Stopped => {
                    if record.retained_cleanup_argv.is_empty() {
                        GcRemoval::Direct
                    } else {
                        GcRemoval::RetainedCleanup {
                            environment_ref: record.environment_ref.clone(),
                        }
                    }
                }
                EnvironmentStatus::Running
                | EnvironmentStatus::Retained
                | EnvironmentStatus::Killing
                | EnvironmentStatus::CleanupFailed => {
                    // The registry believes it live; the runtime's list
                    // says otherwise. Ask the record's own inspect before
                    // calling it an orphan — and even then never remove a
                    // record the registry has not stopped: say so.
                    let verdict = match self.process.observe(record) {
                        EnvironmentLiveness::Running => "its inspect reports it running",
                        EnvironmentLiveness::Gone => {
                            "its inspect reports it gone; kill it (`quecto container kill`) so the record is stopped, then gc"
                        }
                        EnvironmentLiveness::Unknown(_) => "its liveness could not be confirmed",
                    };
                    keep(
                        report,
                        format!(
                            "recorded {} as {} ({container_state}; {verdict})",
                            record.environment_ref,
                            record.status_label()
                        ),
                    );
                    return;
                }
            },
        };
        let reason = match record {
            None => format!("{container_state}; no registry record"),
            Some(record) => format!(
                "{container_state}; recorded {} as stopped",
                record.environment_ref
            ),
        };
        report.removable.push(GcCandidate {
            environment_id: environment_id.to_string(),
            state_dir: state_dir.map(Path::to_path_buf),
            container: container_name.map(str::to_string),
            removal,
            reason,
        });
    }

    fn remove(&self, report: &mut GcReport) {
        let records = self.registry.entries();
        for candidate in report.removable.clone() {
            let outcome = match &candidate.removal {
                GcRemoval::RetainedCleanup { environment_ref } => records
                    .iter()
                    .find(|record| &record.environment_ref == environment_ref)
                    .ok_or_else(|| format!("record {environment_ref} vanished"))
                    .and_then(|record| self.process.cleanup(record))
                    .and_then(|()| self.remove_leftovers(&candidate)),
                GcRemoval::Direct => self.remove_leftovers(&candidate),
            };
            match outcome {
                Ok(()) => report.removed.push(candidate),
                Err(error) => report
                    .errors
                    .push(format!("{}: {error}", candidate.environment_id)),
            }
        }
    }

    /// Remove what is still there after (or instead of) the retained
    /// cleanup: the exited container the runtime still lists, then the
    /// state dir.
    fn remove_leftovers(&self, candidate: &GcCandidate) -> Result<(), String> {
        if let Some(name) = &candidate.container {
            let listed = self
                .inventory
                .containers()?
                .iter()
                .any(|container| &container.name == name);
            if listed {
                self.inventory.remove_container(name)?;
            }
        }
        if let Some(dir) = &candidate.state_dir {
            let root = dir
                .parent()
                .ok_or_else(|| format!("{} has no parent", dir.display()))?;
            self.inventory.remove_environment_dir(root, dir)?;
        }
        Ok(())
    }
}
