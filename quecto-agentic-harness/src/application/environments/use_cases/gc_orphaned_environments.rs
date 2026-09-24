//! Collect orphaned environments (#2024 S4d): `quecto container gc`.
//!
//! An orphan is an environment nothing will ever address again: its
//! runtime container is gone or exited AND the registry either knows
//! nothing about it or records it `stopped`. Everything else is kept and
//! said so — a `running`, `retained`, `killing` or `cleanup-failed` record
//! (its liveness is the kill's to settle, never the collector's), a state
//! dir whose container the config's own `inspect` reports running, a
//! young directory without a container (a create may still be in flight).
//! A `retained` record (#1924) is kept whatever the runtime says of its
//! container: only an explicit `container kill` / `kill_container` moves
//! it to `stopped`, and only then does the collector see it (round 3 H1,
//! #2033). A directory that would otherwise be collected — a `stopped`
//! record's or an unrecorded one — is read for the swarm store its
//! checkout may host (round 4 M1, #2033): one hosting a created run that
//! has not ended, or a store that cannot be read, is kept with the run
//! named, whatever the registry says; so is the signature an older build's
//! restore left on a retained record (round 4 L3). The collector never
//! kills: a stopped record is removed through its own retained `cleanup`
//! (the script set that created it) and then forgotten, an unrecorded
//! orphan through the selected container config's `cleanup` — the
//! harness knows no runtime; the scripts inspect, list and remove. A dry
//! run reports the same judgement without an effect — on the host or on
//! the registry document.
//!
//! Scope is one container config: its own state root (`--state-dir` in
//! its create argv, compared canonically), under which everything is
//! judged, plus — for a record of that config whose workspace lies under
//! a root its *own* retained cleanup names — that record's directory
//! alone, removed through that record's cleanup alone (round 3 M1,
//! #2033). A record's workspace never widens the scan by itself: one
//! that lies anywhere else is reported as outside the config's state dir
//! and nothing is scanned or run for it, so the config's cleanup only
//! ever runs against the root its script knows. Containers the config's
//! `inspect --list` reports (labelled with that root) are judged even
//! when their state dir is already gone.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};
use crate::domain::environment_retention::SwarmRunObservation;

use super::super::dto::{
    AbandonedRuns, ContainerRuntimeTarget, DiagnosableContainerConfig, EnvironmentLiveness,
    GcCandidate, GcKept, GcRefused, GcRemoval, GcReport, GcRequest, RuntimeContainer,
    state_dir_argument,
};
use super::super::ports::{
    ContainerConfigLookup, ContainerRuntimeInventory, EnvironmentProcess, HostedSwarmRunInspection,
};
use super::box_residue::{Residue, ResidueProbe};

/// A directory without a container younger than this is a create that may
/// still be running its clone: kept, never collected.
pub const CREATE_GRACE_SECS: u64 = 15 * 60;

/// What the swarm store below a directory says keeps it.
struct Hosting {
    description: String,
    /// The unfinished run, `id (state)`; `None` for an unreadable store,
    /// which is never an abandoned run.
    run: Option<String>,
}

/// Whether the operator's `--abandoned` policy collects a hosting
/// directory nothing records, and why.
enum Abandoned {
    Collect(String),
    Keep(String),
}

impl Hosting {
    fn abandoned_after(&self, policy: &AbandonedRuns, age_secs: Option<u64>) -> Abandoned {
        let Some(run) = &self.run else {
            return Abandoned::Keep(
                "an unreadable store is never an abandoned run; remove the directory by hand"
                    .to_string(),
            );
        };
        match policy {
            AbandonedRuns::Keep => Abandoned::Keep(
                "end the run, or remove the directory by hand, before it can be collected (or pass --abandoned)"
                    .to_string(),
            ),
            AbandonedRuns::Collect => Abandoned::Collect(format!("{run}, collected on --abandoned")),
            AbandonedRuns::OlderThan { secs, spelled } => match age_secs {
                Some(age) if age >= *secs => Abandoned::Collect(format!(
                    "{run}, {age}s old, collected on --abandoned-after {spelled}"
                )),
                Some(age) => Abandoned::Keep(format!(
                    "the directory is {age}s old, younger than the {spelled} --abandoned-after"
                )),
                None => Abandoned::Keep(format!(
                    "the directory's age could not be read, so it is never older than the {spelled} --abandoned-after"
                )),
            },
        }
    }
}

pub struct GcOrphanedEnvironments {
    registry: EnvironmentRegistry,
    configs: Arc<dyn ContainerConfigLookup>,
    inventory: Arc<dyn ContainerRuntimeInventory>,
    process: Arc<dyn EnvironmentProcess>,
    hosted: Arc<dyn HostedSwarmRunInspection>,
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
        .environment_dir()
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

/// The state root a record's own retained cleanup names (`--state-dir`):
/// the one root beyond the config's a record may vouch for (round 3 M1,
/// #2033), since that is the root its cleanup removes under.
pub fn retained_state_root(record: &EnvironmentRecord) -> Option<PathBuf> {
    state_dir_argument(&record.retained_cleanup_argv)
}

/// Where a record of the collector's config lives relative to its scope.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RecordScope {
    /// Under the config's own state root: judged with everything there.
    ConfigRoot,
    /// Under the root its own retained cleanup names: its directory is
    /// judged alone and removed through that cleanup alone.
    OwnRoot(PathBuf),
    /// Anywhere else — the record vouches for nothing there: reported,
    /// never scanned or removed. Carries where the workspace implies.
    Outside(PathBuf),
}

/// The collector's scope for one run: the config, its root and where
/// each of its records stands.
struct Scope<'a> {
    config: &'a DiagnosableContainerConfig,
    config_root: Option<PathBuf>,
    /// The operator's policy for abandoned runs (#2070).
    abandoned: AbandonedRuns,
    /// By environment ref, for every record of this config.
    records: BTreeMap<&'a str, RecordScope>,
}

impl Scope<'_> {
    fn of(&self, record: &EnvironmentRecord) -> Option<&RecordScope> {
        self.records.get(record.environment_ref.as_str())
    }

    fn root_named(&self) -> String {
        match &self.config_root {
            Some(root) => root.display().to_string(),
            None => "(none named by its create argv)".to_string(),
        }
    }
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
        hosted: Arc<dyn HostedSwarmRunInspection>,
    ) -> Self {
        Self {
            registry,
            configs,
            inventory,
            process,
            hosted,
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

        let scope = self.scope(&config, &records, request.abandoned.clone());
        let mut roots: BTreeSet<PathBuf> = scope.config_root.clone().into_iter().collect();
        roots.extend(scope.records.values().filter_map(|s| match s {
            RecordScope::OwnRoot(root) => Some(root.clone()),
            RecordScope::ConfigRoot | RecordScope::Outside(_) => None,
        }));
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
                let record = by_id.get(dir.environment_id.as_str()).copied();
                // Under a root a record vouched for, that record's
                // directory is the only one judged (round 3 M1): the
                // rest of the root is not this config's.
                if Some(root) != scope.config_root.as_ref()
                    && !record.is_some_and(|record| {
                        scope.of(record) == Some(&RecordScope::OwnRoot(root.clone()))
                    })
                {
                    continue;
                }
                judged.insert(dir.environment_id.clone());
                // The config's own inspect is the authority for a state
                // dir under the config's root (it reads the dir's
                // container and asks the runtime); under a root the record
                // alone vouched for, the record's own retained inspect is
                // (round 4 L2, #2033) — the config's script knows nothing
                // of that root. A dir without a `container` file is not
                // thereby gone (review F4, #2033): the listing may still
                // name a container labelled with its id, and that entry's
                // liveness stands — running is kept, exited is an orphan
                // with the container named. Only a dir the listing knows
                // nothing about is judged container-less.
                let listed = by_environment.get(dir.environment_id.as_str()).copied();
                let own_root = record
                    .filter(|record| matches!(scope.of(record), Some(RecordScope::OwnRoot(_))));
                let liveness = match (own_root, &dir.container, listed) {
                    (Some(record), _, _) => self.process.observe(record),
                    (None, Some(_), _) => self.inventory.inspect(&config, &dir.environment_id),
                    (None, None, Some(container)) if container.running => {
                        EnvironmentLiveness::Running
                    }
                    (None, None, _) => EnvironmentLiveness::Gone,
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
                        record,
                        age_secs: dir.age_secs,
                    },
                    &scope,
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
                &scope,
                &mut report,
            );
        }
        let mut probe = ResidueProbe::new(self.process.as_ref());
        // Records of this config seen nowhere: one outside the scope is
        // said so; a stopped one with nothing left anywhere is what
        // remains, and the record alone is forgotten; any other is kept
        // and said so — never silently skipped (round 4 info, #2033).
        for record in records
            .iter()
            .filter(|record| record.script_name == config.name)
            .filter(|record| judged.insert(record.environment_id.clone()))
        {
            if let Some(RecordScope::Outside(at)) = scope.of(record) {
                report.kept.push(GcKept {
                    environment_id: record.environment_id.clone(),
                    reason: outside_reason(record, at, &scope, "not seen"),
                });
                continue;
            }
            if record.status != EnvironmentStatus::Stopped {
                report.kept.push(GcKept {
                    environment_id: record.environment_id.clone(),
                    reason: format!(
                        "recorded {} as {}; nothing on disk or in the runtime; not collected: kill it (`quecto container kill {}`) so the record is stopped, then gc",
                        record.environment_ref,
                        record.status_label(),
                        record.environment_ref
                    ),
                });
                continue;
            }
            // Seen nowhere by this collection is not yet nothing left: the
            // record's own box is judged as the restore judges it (#2134),
            // so an unscanned root or a container this config's listing
            // misses never loses its record.
            let why_kept = match probe.residue(record) {
                Residue::Nothing => {
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
                    continue;
                }
                Residue::StateOnDisk => "its state directory is still on disk".to_string(),
                Residue::Deferred => {
                    "not inspected this time (the runtime stopped answering, or the inspect budget is spent)".to_string()
                }
                Residue::Kept(reason) => reason,
            };
            report.kept.push(GcKept {
                environment_id: record.environment_id.clone(),
                reason: format!(
                    "recorded {} as stopped; not forgotten: {why_kept}",
                    record.environment_ref
                ),
            });
        }
        if !request.dry_run {
            self.remove(&config, &mut report);
        }
        Ok(report)
    }

    /// Where each record of `config` stands (round 3 M1, #2033): under
    /// the config's root (compared canonically through the host), under
    /// the root its own retained cleanup names, or outside — a workspace
    /// alone vouches for nothing.
    fn scope<'a>(
        &self,
        config: &'a DiagnosableContainerConfig,
        records: &'a [EnvironmentRecord],
        abandoned: AbandonedRuns,
    ) -> Scope<'a> {
        let config_root = config.state_root();
        let canonical = config_root
            .as_deref()
            .map(|root| self.inventory.canonical_root(root));
        let scope_of = |record: &EnvironmentRecord| match implied_state_root(record) {
            Some(root)
                if config_root.as_ref() == Some(&root)
                    || canonical
                        .as_ref()
                        .is_some_and(|c| c == &self.inventory.canonical_root(&root)) =>
            {
                RecordScope::ConfigRoot
            }
            Some(root) if retained_state_root(record).as_ref() == Some(&root) => {
                RecordScope::OwnRoot(root)
            }
            Some(root) => RecordScope::Outside(root),
            None => RecordScope::Outside(record.workspace_path.clone()),
        };
        Scope {
            config,
            records: records
                .iter()
                .filter(|record| record.script_name == config.name)
                .map(|record| (record.environment_ref.as_str(), scope_of(record)))
                .collect(),
            config_root,
            abandoned,
        }
    }

    /// One environment's verdict from its state dir, container and record.
    fn judge(&self, sighting: Sighting<'_>, scope: &Scope<'_>, report: &mut GcReport) {
        let Sighting {
            environment_id,
            state_dir,
            container_name,
            liveness,
            record,
            age_secs,
        } = sighting;
        let config = scope.config;
        let keep = |report: &mut GcReport, reason: String| {
            report.kept.push(GcKept {
                environment_id: environment_id.to_string(),
                reason,
            })
        };
        // A record of this config living outside its scope is reported
        // whatever else was seen of it (round 3 M1): no cleanup — its own
        // or the config's — runs against a root nobody vouched for.
        if let Some(record) = record
            && let Some(RecordScope::Outside(at)) = scope.of(record)
        {
            let seen = match (&liveness, container_name) {
                (EnvironmentLiveness::Running, Some(name)) => format!("container {name} running"),
                (EnvironmentLiveness::Running, None) => "reported running".to_string(),
                (EnvironmentLiveness::Gone, Some(name)) => {
                    format!("container {name} gone or exited")
                }
                (EnvironmentLiveness::Gone, None) => "no container".to_string(),
                (EnvironmentLiveness::Unknown(reason), _) => format!("liveness unknown: {reason}"),
            };
            keep(report, outside_reason(record, at, scope, &seen));
            return;
        }
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
        // A directory whose age cannot be read counts as young (round 2
        // F-F, #2033): the grace protects a create in flight, and not
        // knowing the age is no evidence there is none.
        if container_name.is_none()
            && record.is_none()
            && age_secs.is_none_or(|age| age < CREATE_GRACE_SECS)
        {
            let age = match age_secs {
                Some(age) => format!("the directory is {age}s old"),
                None => "the directory's age could not be read".to_string(),
            };
            keep(
                report,
                format!(
                    "no container recorded yet and {age}: a create may be in flight (older than {CREATE_GRACE_SECS}s it is collected)"
                ),
            );
            return;
        }
        let removal = match record {
            None => GcRemoval::ConfiguredCleanup {
                config: config.name.clone(),
            },
            // An older build's restore relabelled a retained record
            // `stopped` (round 4 L3): its own `retained` reason under that
            // status with the restore's last error is the signature; this
            // build's restore puts it back, and the collector never takes
            // it meanwhile.
            Some(record) if record.relabelled_while_retained() => {
                keep(
                    report,
                    format!(
                        "{container_state}; recorded {} as stopped, but it was retained; relabelled by an older build — kill explicitly to collect (`quecto container ls` restores it to retained first)",
                        record.environment_ref
                    ),
                );
                return;
            }
            Some(record) => match record.status {
                EnvironmentStatus::Stopped => {
                    // Under a root the record alone vouched for, only its
                    // own cleanup may remove (round 3 M1); the config's
                    // cleanup serves a record without one under the
                    // config's own root only.
                    let own_root = matches!(scope.of(record), Some(RecordScope::OwnRoot(_)));
                    debug_assert!(!own_root || !record.retained_cleanup_argv.is_empty());
                    if record.retained_cleanup_argv.is_empty() && !own_root {
                        GcRemoval::ConfiguredCleanup {
                            config: config.name.clone(),
                        }
                    } else {
                        GcRemoval::RetainedCleanup {
                            environment_ref: record.environment_ref.clone(),
                        }
                    }
                }
                // Retained (#1924): kept with its state dir — board,
                // checkout, unpushed work — whatever the runtime says of
                // its container (under the shipped adapter it has exited
                // by design). Only an explicit kill moves it to `stopped`;
                // the collector never does (round 3 H1, #2033).
                EnvironmentStatus::Retained => {
                    keep(
                        report,
                        format!(
                            "recorded {} as retained ({container_state}); kept with its state dir until an explicit kill (`quecto container kill {}`)",
                            record.environment_ref, record.environment_ref
                        ),
                    );
                    return;
                }
                EnvironmentStatus::Running
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
        // What the registry calls collectable may still be a swarm's
        // board and checkout (round 4 M1): the master exited before its
        // coordinator, nobody retained the box, and the store under the
        // directory says the run is not over. Read last — only for what
        // would otherwise go — and kept with the run named, unless nothing
        // records it and the operator asked for abandoned runs (#2070).
        let mut reason = reason;
        if let Some(hosting) = self.hosted_run_keeps(record, state_dir) {
            match (record, hosting.abandoned_after(&scope.abandoned, age_secs)) {
                (None, Abandoned::Collect(why)) => {
                    reason = format!("{reason}; its checkout hosts abandoned swarm run {why}");
                }
                (None, Abandoned::Keep(why)) => {
                    keep(
                        report,
                        format!(
                            "{reason}, but its checkout {}; nothing records it: {why}",
                            hosting.description
                        ),
                    );
                    return;
                }
                (Some(_), _) => {
                    keep(
                        report,
                        format!(
                            "{reason}, but its checkout {}; end the run (or remove the directory by hand if its board is unreadable) before it can be collected",
                            hosting.description
                        ),
                    );
                    return;
                }
            }
        }
        report.removable.push(GcCandidate {
            environment_id: environment_id.to_string(),
            state_dir: state_dir.map(Path::to_path_buf),
            container: container_name.map(str::to_string),
            removal,
            reason,
        });
    }

    /// Why the swarm store below a directory keeps it, when it does: it
    /// hosts a created run its owner has not closed — the same rule the
    /// finalizer keeps a box by (#2070), whatever the run's status — or a
    /// store that cannot be read (kept likewise: a destroyed box cannot be
    /// recovered). Read through the record when there is one (its
    /// advertised checkout), else below the bare state dir.
    fn hosted_run_keeps(
        &self,
        record: Option<&EnvironmentRecord>,
        state_dir: Option<&Path>,
    ) -> Option<Hosting> {
        let observed = match (record, state_dir) {
            (Some(record), _) => self.hosted.inspect_hosted_run(record),
            (None, Some(dir)) => self.hosted.inspect_hosted_run_at(dir),
            (None, None) => return None,
        };
        match observed {
            SwarmRunObservation::Run(run) if run.keeps_environment() => Some(Hosting {
                description: format!("hosts swarm run {} ({})", run.id, run.describe()),
                run: Some(format!("{} ({})", run.id, run.describe())),
            }),
            SwarmRunObservation::Run(_) | SwarmRunObservation::NoStore => None,
            SwarmRunObservation::Unreadable(error) => Some(Hosting {
                description: format!("hosts a coordination store that could not be read ({error})"),
                run: None,
            }),
        }
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

/// Why a record outside the collector's scope is kept.
fn outside_reason(record: &EnvironmentRecord, at: &Path, scope: &Scope<'_>, seen: &str) -> String {
    format!(
        "recorded {} at {} ({seen}); outside this config's state dir {}; not collected",
        record.environment_ref,
        at.display(),
        scope.root_named()
    )
}
