//! Collect orphaned environments (#2024 S4d): `quecto container gc`.
//!
//! An orphan is an environment nothing will ever address again: its
//! runtime container is gone or exited AND the registry either knows
//! nothing about it or records it `stopped`. Everything else is kept and
//! said so — a `running`, `retained`, `killing` or `cleanup-failed` record
//! (its liveness is the kill's to settle, never the collector's), a state
//! dir whose container the config's own `inspect` reports running, a
//! young directory without a container (a create may still be in flight).
//! The collector never kills: a stopped record is removed through its own
//! retained `cleanup` (the script set that created it) and then forgotten,
//! an unrecorded orphan through the selected container config's `cleanup`
//! — the harness knows no runtime; the scripts inspect, list and remove.
//! A dry run reports the same judgement without an effect.
//!
//! Scope is one container config: its own state root (`--state-dir` in
//! its create argv) plus the roots implied by the records that config
//! created. Nothing under another config's root is judged, so a cleanup
//! always runs against the root its script knows. Containers the config's
//! `inspect --list` reports (labelled with that root) are judged even when
//! their state dir is already gone.
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

/// A directory without a container younger than this is a create that may
/// still be running its clone: kept, never collected.
pub const CREATE_GRACE_SECS: u64 = 15 * 60;

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
    /// What the runtime says: the config's `inspect` for a directory, the
    /// listing's entry for a directory-less container.
    liveness: EnvironmentLiveness,
    record: Option<&'a EnvironmentRecord>,
    /// Seconds since the state dir changed, when it exists and is known.
    age_secs: Option<u64>,
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

        let mut roots: BTreeSet<PathBuf> = config.state_root().into_iter().collect();
        roots.extend(
            records
                .iter()
                .filter(|record| record.script_name == config.name)
                .filter_map(implied_state_root),
        );
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
                // The config's own inspect is the authority for a state
                // dir (it reads the dir's container and asks the runtime).
                // A dir without a `container` file is not thereby gone
                // (review F4, #2033): the listing may still name a
                // container labelled with its id, and that entry's
                // liveness stands — running is kept, exited is an orphan
                // with the container named. Only a dir the listing knows
                // nothing about is judged container-less.
                let listed = by_environment.get(dir.environment_id.as_str()).copied();
                let liveness = match (&dir.container, listed) {
                    (Some(_), _) => self.inventory.inspect(&config, &dir.environment_id),
                    (None, Some(container)) if container.running => EnvironmentLiveness::Running,
                    (None, _) => EnvironmentLiveness::Gone,
                };
                let container_name = dir
                    .container
                    .as_deref()
                    .or(listed.map(|c| c.container.as_str()));
                self.judge(
                    Sighting {
                        environment_id: &dir.environment_id,
                        state_dir: Some(&dir.path),
                        container_name,
                        liveness,
                        record: by_id.get(dir.environment_id.as_str()).copied(),
                        age_secs: dir.age_secs,
                    },
                    &config,
                    &mut report,
                );
            }
        }
        // Containers whose state dir is already gone.
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
                    liveness: if container.running {
                        EnvironmentLiveness::Running
                    } else {
                        EnvironmentLiveness::Gone
                    },
                    record: by_id.get(container.environment_id.as_str()).copied(),
                    age_secs: None,
                },
                &config,
                &mut report,
            );
        }
        // Stopped records of this config with nothing left anywhere: the
        // record alone is what remains.
        for record in records
            .iter()
            .filter(|record| record.script_name == config.name)
            .filter(|record| record.status == EnvironmentStatus::Stopped)
            .filter(|record| judged.insert(record.environment_id.clone()))
        {
            report.removable.push(GcCandidate {
                environment_id: record.environment_id.clone(),
                state_dir: None,
                container: None,
                removal: GcRemoval::ForgetRecord {
                    environment_ref: record.environment_ref.clone(),
                },
                reason: format!(
                    "nothing on disk or in the runtime; recorded {} as stopped",
                    record.environment_ref
                ),
            });
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
            liveness,
            record,
            age_secs,
        } = sighting;
        let keep = |report: &mut GcReport, reason: String| {
            report.kept.push(GcKept {
                environment_id: environment_id.to_string(),
                reason,
            })
        };
        let container_state = match (&liveness, container_name) {
            (EnvironmentLiveness::Running, Some(name)) => {
                keep(report, format!("container {name} is running"));
                return;
            }
            (EnvironmentLiveness::Running, None) => {
                keep(report, "its inspect reports it running".to_string());
                return;
            }
            (EnvironmentLiveness::Unknown(reason), _) => {
                keep(
                    report,
                    format!("its liveness could not be confirmed: {reason}"),
                );
                return;
            }
            (EnvironmentLiveness::Gone, Some(name)) => format!("container {name} gone or exited"),
            (EnvironmentLiveness::Gone, None) => "no container recorded".to_string(),
        };
        if container_name.is_none()
            && record.is_none()
            && age_secs.is_some_and(|age| age < CREATE_GRACE_SECS)
        {
            keep(
                report,
                format!(
                    "no container recorded yet and the directory is {}s old: a create may be in flight (older than {CREATE_GRACE_SECS}s it is collected)",
                    age_secs.unwrap_or(0)
                ),
            );
            return;
        }
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
                    // the registry has not stopped: say what its own
                    // inspect thinks and point at the kill.
                    let verdict = match self.process.observe(record) {
                        EnvironmentLiveness::Running => "its retained inspect reports it running",
                        EnvironmentLiveness::Gone => {
                            "its retained inspect reports it gone; kill it (`quecto container kill`) so the record is stopped, then gc"
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
                    .and_then(|record| self.process.cleanup(record))
                    .and_then(|()| self.leftovers(&candidate))
                    .map(|()| {
                        // Collected: the record has nothing left to name.
                        self.registry.remove(environment_ref);
                    }),
                GcRemoval::ConfiguredCleanup { .. } => self
                    .inventory
                    .remove(config, &candidate.environment_id)
                    .and_then(|()| self.leftovers(&candidate)),
                GcRemoval::ForgetRecord { environment_ref } => {
                    self.registry.remove(environment_ref);
                    Ok(())
                }
            };
            match outcome {
                Ok(()) => report.removed.push(candidate),
                Err(error) => report
                    .errors
                    .push(format!("{}: {error}", candidate.environment_id)),
            }
        }
    }

    /// The script owns the layout: a cleanup that leaves the directory
    /// behind is reported, never finished by hand.
    fn leftovers(&self, candidate: &GcCandidate) -> Result<(), String> {
        match &candidate.state_dir {
            Some(dir) if self.dir_still_listed(dir) => Err(format!(
                "cleanup ran but {} is still there; remove it by hand or fix the script",
                dir.display()
            )),
            _ => Ok(()),
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
