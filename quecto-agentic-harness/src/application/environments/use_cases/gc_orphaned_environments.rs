//! Collect orphaned environments (#2024 S4d): `quecto container gc`.
//!
//! An orphan is an environment nothing will ever address again: its
//! runtime container is gone or exited AND the registry either knows
//! nothing about it or records it `stopped`. Everything else is kept and
//! said so — a `running`, `retained`, `killing` or `cleanup-failed` record
//! (its liveness is the kill's to settle, never the collector's), an
//! unknown state dir whose container still runs. The collector never
//! kills: a stopped record is removed through its own retained `cleanup`
//! (the script set that created it), an unrecorded orphan through the
//! selected container config's `cleanup` — the harness knows no runtime;
//! the scripts list (`inspect --list`) and remove. A dry run reports the
//! same judgement without an effect.
//!
//! The state roots scanned are the config's own (`--state-dir` in its
//! create argv), the ones the request names and the ones the registry's
//! records imply (the parent of each record's environment directory);
//! containers the runtime lists are judged even when their state dir is
//! already gone.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};

use super::super::dto::{
    ContainerRuntimeTarget, DiagnosableContainerConfig, EnvironmentLiveness, GcCandidate, GcKept,
    GcRefused, GcRemoval, GcReport, GcRequest, RuntimeContainer,
};
use super::super::ports::{ContainerConfigLookup, ContainerRuntimeInventory, EnvironmentProcess};

pub struct GcOrphanedEnvironments {
    registry: EnvironmentRegistry,
    configs: Arc<dyn ContainerConfigLookup>,
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
        configs: Arc<dyn ContainerConfigLookup>,
        inventory: Arc<dyn ContainerRuntimeInventory>,
        process: Arc<dyn EnvironmentProcess>,
    ) -> Self {
        Self {
            registry,
            configs,
            inventory,
            process,
        }
    }

    pub fn execute(&self, request: &GcRequest) -> Result<GcReport, GcRefused> {
        let config = self
            .configs
            .lookup(&ContainerRuntimeTarget {
                name: request.config.clone(),
            })
            .map_err(GcRefused)?;
        if config.inspect.is_empty() || config.cleanup.is_empty() {
            return Err(GcRefused(format!(
                "container config '{}' has no inspect or cleanup script: the collector cannot list or remove environments through it",
                config.name
            )));
        }
        let mut report = GcReport {
            dry_run: request.dry_run,
            config: config.name.clone(),
            ..GcReport::default()
        };
        let records = self.registry.entries();
        let by_id: BTreeMap<&str, &EnvironmentRecord> = records
            .iter()
            .map(|record| (record.environment_id.as_str(), record))
            .collect();

        let mut roots: BTreeSet<PathBuf> = request.state_roots.iter().cloned().collect();
        roots.extend(config.state_root());
        roots.extend(records.iter().filter_map(implied_state_root));
        report.state_roots = roots.iter().cloned().collect();

        let containers = match self.inventory.containers(&config) {
            Ok(containers) => containers,
            Err(error) => {
                // Without the runtime's word nothing can be called exited:
                // judge nothing, say why.
                report
                    .errors
                    .push(format!("container runtime inventory unavailable: {error}"));
                return Ok(report);
            }
        };
        let by_container: BTreeMap<&str, &RuntimeContainer> = containers
            .iter()
            .map(|container| (container.container.as_str(), container))
            .collect();
        let by_environment: BTreeMap<&str, &RuntimeContainer> = containers
            .iter()
            .map(|container| (container.environment_id.as_str(), container))
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
                // The runtime's entry: by the container the dir names, or
                // by the environment id it was labelled with.
                let container = dir
                    .container
                    .as_deref()
                    .and_then(|name| by_container.get(name).copied())
                    .or_else(|| by_environment.get(dir.environment_id.as_str()).copied());
                self.judge(
                    Sighting {
                        environment_id: &dir.environment_id,
                        state_dir: Some(&dir.path),
                        container_name: dir
                            .container
                            .as_deref()
                            .or(container.map(|c| c.container.as_str())),
                        container,
                        record: by_id.get(dir.environment_id.as_str()).copied(),
                    },
                    &config,
                    &mut report,
                );
            }
        }
        // Containers whose state dir is already gone (or lives under a
        // root nobody named).
        for container in &containers {
            if container.environment_id.is_empty()
                || !judged.insert(container.environment_id.clone())
            {
                continue;
            }
            self.judge(
                Sighting {
                    environment_id: &container.environment_id,
                    state_dir: None,
                    container_name: Some(container.container.as_str()),
                    container: Some(container),
                    record: by_id.get(container.environment_id.as_str()).copied(),
                },
                &config,
                &mut report,
            );
        }
        if !request.dry_run {
            self.remove(&config, &mut report);
        }
        Ok(report)
    }

    /// One environment's verdict from its state dir, container and record.
    fn judge(
        &self,
        sighting: Sighting<'_>,
        config: &DiagnosableContainerConfig,
        report: &mut GcReport,
    ) {
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
            keep(
                report,
                format!("container {} is running", container.container),
            );
            return;
        }
        let container_state = match (container_name, container) {
            (Some(name), Some(_)) => format!("container {name} exited"),
            (Some(name), None) => format!("container {name} not known to the runtime"),
            (None, _) => "no container recorded".to_string(),
        };
        let removal = match record {
            None => GcRemoval::ConfiguredCleanup {
                config: config.name.clone(),
            },
            Some(record) => match record.status {
                EnvironmentStatus::Stopped => {
                    if record.retained_cleanup_argv.is_empty() {
                        GcRemoval::ConfiguredCleanup {
                            config: config.name.clone(),
                        }
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
                    // The registry believes it live. Never remove a record
                    // the registry has not stopped: say what the runtime
                    // and its own inspect think and point at the kill.
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

    fn remove(&self, config: &DiagnosableContainerConfig, report: &mut GcReport) {
        let records = self.registry.entries();
        for candidate in report.removable.clone() {
            let outcome = match &candidate.removal {
                GcRemoval::RetainedCleanup { environment_ref } => records
                    .iter()
                    .find(|record| &record.environment_ref == environment_ref)
                    .ok_or_else(|| format!("record {environment_ref} vanished"))
                    .and_then(|record| self.process.cleanup(record)),
                GcRemoval::ConfiguredCleanup { .. } => {
                    self.inventory.remove(config, &candidate.environment_id)
                }
            };
            let outcome = outcome.and_then(|()| match &candidate.state_dir {
                // The script owns the layout: a cleanup that leaves the
                // directory behind is reported, never finished by hand.
                Some(dir) if self.dir_still_listed(dir) => Err(format!(
                    "cleanup ran but {} is still there; remove it by hand or fix the script",
                    dir.display()
                )),
                _ => Ok(()),
            });
            match outcome {
                Ok(()) => report.removed.push(candidate),
                Err(error) => report
                    .errors
                    .push(format!("{}: {error}", candidate.environment_id)),
            }
        }
    }

    /// Whether the root still lists `dir` after a removal (through the
    /// inventory port: the application reads no filesystem).
    fn dir_still_listed(&self, dir: &Path) -> bool {
        let Some(root) = dir.parent() else {
            return false;
        };
        self.inventory
            .environment_dirs(root)
            .map(|dirs| dirs.iter().any(|listed| listed.path == dir))
            .unwrap_or(false)
    }
}
