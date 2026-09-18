use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::super::dto::{
    ContainerRuntimeTarget, DiagnosableContainerConfig, EnvironmentLiveness, EnvironmentStateDir,
    GcRemoval, GcRequest, RuntimeContainer,
};
use super::super::ports::{ContainerConfigLookup, ContainerRuntimeInventory, EnvironmentProcess};
use super::{CREATE_GRACE_SECS, GcOrphanedEnvironments, implied_state_root};
use crate::domain::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};

struct FakeLookup(Result<DiagnosableContainerConfig, String>);

impl ContainerConfigLookup for FakeLookup {
    fn lookup(
        &self,
        target: &ContainerRuntimeTarget,
    ) -> Result<DiagnosableContainerConfig, String> {
        match (&self.0, &target.name) {
            (Ok(config), Some(name)) if name != &config.name => {
                Err(format!("unknown container config '{name}'"))
            }
            (result, _) => result.clone(),
        }
    }
}

/// The fake host: state dirs with their inspect verdict, the listing,
/// what was removed.
#[derive(Default)]
struct FakeInventory {
    containers: Mutex<Vec<RuntimeContainer>>,
    dirs: Mutex<Vec<EnvironmentStateDir>>,
    /// The config's inspect verdict per environment id (default: gone).
    liveness: Mutex<Vec<(String, EnvironmentLiveness)>>,
    fail_list: bool,
    removed: Mutex<Vec<String>>,
    /// Ids whose cleanup leaves the dir behind.
    stubborn: Vec<String>,
}

impl ContainerRuntimeInventory for FakeInventory {
    fn containers(
        &self,
        _config: &DiagnosableContainerConfig,
    ) -> Result<Vec<RuntimeContainer>, String> {
        if self.fail_list {
            return Err("inspect --list unsupported".into());
        }
        Ok(self.containers.lock().unwrap().clone())
    }
    fn inspect(&self, _config: &DiagnosableContainerConfig, id: &str) -> EnvironmentLiveness {
        self.liveness
            .lock()
            .unwrap()
            .iter()
            .find(|(seen, _)| seen == id)
            .map(|(_, l)| l.clone())
            .unwrap_or(EnvironmentLiveness::Gone)
    }
    fn environment_dirs(&self, root: &Path) -> Result<Vec<EnvironmentStateDir>, String> {
        if root == Path::new("/unreadable") {
            return Err("permission denied".into());
        }
        let mut dirs: Vec<EnvironmentStateDir> = self
            .dirs
            .lock()
            .unwrap()
            .iter()
            .filter(|dir| dir.path.parent() == Some(root))
            .cloned()
            .collect();
        dirs.sort_by(|a, b| a.environment_id.cmp(&b.environment_id));
        Ok(dirs)
    }
    fn remove(&self, _config: &DiagnosableContainerConfig, id: &str) -> Result<(), String> {
        if id == "env-broken" {
            return Err("cleanup exited 1".into());
        }
        self.removed.lock().unwrap().push(id.into());
        self.containers
            .lock()
            .unwrap()
            .retain(|c| c.environment_id != id);
        if !self.stubborn.iter().any(|s| s == id) {
            self.dirs.lock().unwrap().retain(|d| d.environment_id != id);
        }
        Ok(())
    }
}

#[derive(Default)]
struct FakeProcess {
    liveness: Mutex<Vec<(String, EnvironmentLiveness)>>,
    cleaned: Mutex<Vec<String>>,
    host: Option<Arc<FakeInventory>>,
}

impl EnvironmentProcess for FakeProcess {
    fn observe(&self, record: &EnvironmentRecord) -> EnvironmentLiveness {
        self.liveness
            .lock()
            .unwrap()
            .iter()
            .find(|(r, _)| r == &record.environment_ref)
            .map(|(_, l)| l.clone())
            .unwrap_or(EnvironmentLiveness::Unknown("unset".into()))
    }
    fn cleanup(&self, record: &EnvironmentRecord) -> Result<(), String> {
        self.cleaned
            .lock()
            .unwrap()
            .push(record.environment_ref.clone());
        if let Some(host) = &self.host {
            host.dirs
                .lock()
                .unwrap()
                .retain(|d| d.environment_id != record.environment_id);
            host.containers
                .lock()
                .unwrap()
                .retain(|c| c.environment_id != record.environment_id);
        }
        Ok(())
    }
}

fn config() -> DiagnosableContainerConfig {
    DiagnosableContainerConfig {
        name: "box".into(),
        create: vec!["create".into(), "--state-dir".into(), "/s".into()],
        inspect: vec!["inspect".into()],
        cleanup: vec!["cleanup".into()],
        diagnostics: vec![],
    }
}

fn record(reference: &str, id: &str, status: EnvironmentStatus) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: reference.into(),
        environment_id: id.into(),
        environment_uuid: format!("uuid-{reference}"),
        name: None,
        workspace_path: PathBuf::from("/s").join(id).join("workspace"),
        repository: String::new(),
        script_name: "box".into(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec![],
        retained_cleanup_argv: vec!["cleanup".into()],
        retained_inspect_argv: vec!["inspect".into()],
        members: vec![],
        status,
        metadata: serde_json::json!({}),
        last_error: None,
        origin: EnvironmentOrigin::Restored,
        created_by: String::new(),
        created_at: None,
    }
}

fn dir(id: &str, container: Option<&str>) -> EnvironmentStateDir {
    EnvironmentStateDir {
        path: PathBuf::from("/s").join(id),
        environment_id: id.into(),
        container: container.map(str::to_string),
        age_secs: Some(CREATE_GRACE_SECS * 2),
    }
}

fn container(id: &str, running: bool) -> RuntimeContainer {
    RuntimeContainer {
        environment_id: id.into(),
        container: format!("quecto-{id}"),
        running,
    }
}

struct Rig {
    registry: EnvironmentRegistry,
    host: Arc<FakeInventory>,
    process: Arc<FakeProcess>,
    lookup: Result<DiagnosableContainerConfig, String>,
}

impl Rig {
    fn new() -> Self {
        Self::over(FakeInventory::default())
    }
    fn over(host: FakeInventory) -> Self {
        let host = Arc::new(host);
        Self {
            registry: EnvironmentRegistry::new(),
            process: Arc::new(FakeProcess {
                host: Some(host.clone()),
                ..Default::default()
            }),
            host,
            lookup: Ok(config()),
        }
    }
    fn use_case(&self) -> GcOrphanedEnvironments {
        GcOrphanedEnvironments::new(
            self.registry.clone(),
            Arc::new(FakeLookup(self.lookup.clone())),
            self.host.clone(),
            self.process.clone(),
        )
    }
    fn dry_run(&self) -> super::super::dto::GcReport {
        self.use_case()
            .execute(&GcRequest {
                dry_run: true,
                config: None,
            })
            .unwrap()
    }
}

#[test]
fn implied_state_root_is_the_parent_of_the_environment_directory() {
    let mut record = record("C1", "env-a", EnvironmentStatus::Running);
    assert_eq!(implied_state_root(&record), Some(PathBuf::from("/s")));
    record.workspace_path = "/s/env-a/workspace/repo".into();
    assert_eq!(implied_state_root(&record), Some(PathBuf::from("/s")));
    record.workspace_path = "/elsewhere/workspace".into();
    assert_eq!(implied_state_root(&record), None);
}

#[test]
fn the_configs_state_root_is_read_from_its_create_argv() {
    assert_eq!(config().state_root(), Some(PathBuf::from("/s")));
    let mut bare = config();
    bare.create = vec![
        "create".into(),
        "--".into(),
        "--state-dir".into(),
        "/x".into(),
    ];
    assert_eq!(bare.state_root(), None);
    bare.create = vec!["create".into(), "--state-dir".into()];
    assert_eq!(bare.state_root(), None);
}

#[test]
fn orphans_are_gone_environments_with_no_record_or_a_stopped_one_and_the_rest_is_kept() {
    let rig = Rig::new();
    rig.registry
        .commit(record("C1", "env-live", EnvironmentStatus::Running));
    rig.registry
        .commit(record("C2", "env-stopped", EnvironmentStatus::Stopped));
    rig.registry
        .commit(record("C3", "env-believed", EnvironmentStatus::Retained));
    // A record of another config under another root: not this collector's.
    let mut other = record("C4", "env-other", EnvironmentStatus::Stopped);
    other.script_name = "other".into();
    other.workspace_path = "/t/env-other/workspace".into();
    rig.registry.commit(other);
    *rig.process.liveness.lock().unwrap() = vec![("C3".into(), EnvironmentLiveness::Gone)];
    *rig.host.liveness.lock().unwrap() = vec![
        ("env-live".into(), EnvironmentLiveness::Running),
        ("env-unlabelled-live".into(), EnvironmentLiveness::Running),
        (
            "env-unsure".into(),
            EnvironmentLiveness::Unknown("daemon down".into()),
        ),
    ];
    *rig.host.dirs.lock().unwrap() = vec![
        dir("env-live", Some("quecto-env-live")),
        dir("env-stopped", Some("quecto-env-stopped")),
        dir("env-believed", Some("quecto-env-believed")),
        dir("env-orphan", Some("quecto-env-orphan")),
        dir("env-nocontainer", None),
        dir("env-unlabelled-live", Some("quecto-env-unlabelled-live")),
        dir("env-unsure", Some("quecto-env-unsure")),
        EnvironmentStateDir {
            path: "/t/env-other".into(),
            environment_id: "env-other".into(),
            container: None,
            age_secs: Some(CREATE_GRACE_SECS * 2),
        },
    ];
    *rig.host.containers.lock().unwrap() = vec![
        container("env-live", true),
        container("env-orphan", false),
        container("env-ghost", false),
        container("env-ghost-running", true),
    ];
    let report = rig.dry_run();
    assert!(report.dry_run);
    assert_eq!(report.config, "box");
    assert_eq!(report.state_roots, vec![PathBuf::from("/s")]);
    let removable: Vec<(&str, &GcRemoval)> = report
        .removable
        .iter()
        .map(|c| (c.environment_id.as_str(), &c.removal))
        .collect();
    let configured = GcRemoval::ConfiguredCleanup {
        config: "box".into(),
    };
    assert_eq!(
        removable,
        vec![
            ("env-nocontainer", &configured),
            ("env-orphan", &configured),
            (
                "env-stopped",
                &GcRemoval::RetainedCleanup {
                    environment_ref: "C2".into()
                }
            ),
            ("env-ghost", &configured),
        ]
    );
    let ghost = &report.removable[3];
    assert_eq!(ghost.state_dir, None);
    assert_eq!(ghost.container.as_deref(), Some("quecto-env-ghost"));
    let kept: Vec<(&str, &str)> = report
        .kept
        .iter()
        .map(|k| (k.environment_id.as_str(), k.reason.as_str()))
        .collect();
    assert_eq!(kept.len(), 5, "{kept:?}");
    assert_eq!(kept[0].0, "env-believed");
    assert!(kept[0].1.contains("recorded C3 as retained"), "{kept:?}");
    assert!(kept[0].1.contains("quecto container kill"), "{kept:?}");
    assert_eq!(
        kept[1],
        ("env-live", "container quecto-env-live is running")
    );
    assert_eq!(
        kept[2],
        (
            "env-unlabelled-live",
            "container quecto-env-unlabelled-live is running"
        ),
        "the config's inspect, not the listing, decides a directory"
    );
    assert!(
        kept[3].1.contains("could not be confirmed: daemon down"),
        "{kept:?}"
    );
    assert_eq!(
        kept[4],
        (
            "env-ghost-running",
            "container quecto-env-ghost-running is running"
        )
    );
    assert!(report.removed.is_empty() && report.errors.is_empty());
    assert!(rig.host.removed.lock().unwrap().is_empty());
    assert!(rig.process.cleaned.lock().unwrap().is_empty());
}

#[test]
fn a_young_directory_without_a_container_is_a_create_in_flight_and_kept() {
    let rig = Rig::new();
    let mut young = dir("env-fresh", None);
    young.age_secs = Some(30);
    let mut unknown_age = dir("env-ageless", None);
    unknown_age.age_secs = None;
    *rig.host.dirs.lock().unwrap() = vec![young, unknown_age, dir("env-old", None)];
    let report = rig.dry_run();
    let removable: Vec<&str> = report
        .removable
        .iter()
        .map(|c| c.environment_id.as_str())
        .collect();
    assert_eq!(removable, ["env-ageless", "env-old"]);
    assert_eq!(report.kept.len(), 1);
    assert_eq!(report.kept[0].environment_id, "env-fresh");
    assert!(
        report.kept[0].reason.contains("a create may be in flight"),
        "{:?}",
        report.kept
    );
}

#[test]
fn a_real_run_removes_through_the_right_cleanup_forgets_collected_records_and_reports_leftovers() {
    let rig = Rig::over(FakeInventory {
        stubborn: vec!["env-stubborn".into()],
        ..Default::default()
    });
    rig.registry
        .commit(record("C2", "env-stopped", EnvironmentStatus::Stopped));
    let mut no_cleanup = record("C4", "env-bare", EnvironmentStatus::Stopped);
    no_cleanup.retained_cleanup_argv.clear();
    rig.registry.commit(no_cleanup);
    // Stopped with nothing left anywhere: the record alone is forgotten.
    rig.registry
        .commit(record("C5", "env-memory", EnvironmentStatus::Stopped));
    *rig.host.dirs.lock().unwrap() = vec![
        dir("env-stopped", Some("quecto-env-stopped")),
        dir("env-bare", None),
        dir("env-orphan", Some("quecto-env-orphan")),
        dir("env-broken", None),
        dir("env-stubborn", None),
    ];
    *rig.host.containers.lock().unwrap() = vec![container("env-orphan", false)];
    let report = rig.use_case().execute(&GcRequest::default()).unwrap();
    let removed: Vec<(&str, &GcRemoval)> = report
        .removed
        .iter()
        .map(|c| (c.environment_id.as_str(), &c.removal))
        .collect();
    assert_eq!(
        removed.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        ["env-bare", "env-orphan", "env-stopped", "env-memory"]
    );
    assert_eq!(
        removed[3].1,
        &GcRemoval::ForgetRecord {
            environment_ref: "C5".into()
        }
    );
    assert_eq!(rig.process.cleaned.lock().unwrap().as_slice(), ["C2"]);
    assert_eq!(
        rig.host.removed.lock().unwrap().as_slice(),
        ["env-bare", "env-orphan", "env-stubborn"]
    );
    // Collected records are forgotten; the failed ones stay.
    let remaining: Vec<String> = rig
        .registry
        .entries()
        .into_iter()
        .map(|r| r.environment_ref)
        .collect();
    assert_eq!(
        remaining,
        ["C4"],
        "C4 had no retained cleanup: the config's ran, the record stays stopped"
    );
    assert_eq!(report.errors.len(), 2, "{:?}", report.errors);
    assert!(
        report.errors[0].starts_with("env-broken: cleanup exited 1"),
        "{:?}",
        report.errors
    );
    assert!(
        report.errors[1].contains("env-stubborn: cleanup ran but /s/env-stubborn is still there"),
        "{:?}",
        report.errors
    );
}

#[test]
fn a_live_record_is_kept_whatever_the_runtime_lists() {
    let rig = Rig::new();
    rig.registry
        .commit(record("C1", "env-live", EnvironmentStatus::CleanupFailed));
    *rig.host.dirs.lock().unwrap() = vec![dir("env-live", Some("quecto-env-live"))];
    *rig.process.liveness.lock().unwrap() = vec![("C1".into(), EnvironmentLiveness::Running)];
    let report = rig.dry_run();
    assert!(report.removable.is_empty());
    assert!(
        report.kept[0]
            .reason
            .contains("recorded C1 as cleanup-failed"),
        "{:?}",
        report.kept
    );
    assert!(
        report.kept[0]
            .reason
            .contains("retained inspect reports it running"),
        "{:?}",
        report.kept
    );
}

#[test]
fn an_unavailable_inventory_judges_nothing_and_an_unreadable_root_is_reported() {
    let rig = Rig::over(FakeInventory {
        fail_list: true,
        ..Default::default()
    });
    let report = rig.dry_run();
    assert!(report.removable.is_empty() && report.kept.is_empty());
    assert_eq!(report.errors.len(), 1);
    assert!(report.errors[0].contains("inspect --list unsupported"));

    let mut rig = Rig::new();
    let mut config = config();
    config.create = vec!["create".into(), "--state-dir".into(), "/unreadable".into()];
    rig.lookup = Ok(config);
    let report = rig.dry_run();
    assert_eq!(report.errors.len(), 1);
    assert!(
        report.errors[0].contains("state root /unreadable not scanned: permission denied"),
        "{:?}",
        report.errors
    );
}

#[test]
fn the_collector_refuses_without_a_usable_config() {
    let mut rig = Rig::new();
    rig.lookup = Err("no container_configs".into());
    let refused = rig.use_case().execute(&GcRequest::default()).unwrap_err();
    assert_eq!(refused.0, "no container_configs");
    assert_eq!(refused.to_string(), "no container_configs");

    let mut rig = Rig::new();
    let mut config = config();
    config.inspect.clear();
    rig.lookup = Ok(config);
    let refused = rig.use_case().execute(&GcRequest::default()).unwrap_err();
    assert!(
        refused.0.contains("has no inspect or cleanup script"),
        "{refused}"
    );

    let rig = Rig::new();
    let refused = rig
        .use_case()
        .execute(&GcRequest {
            dry_run: true,
            config: Some("other".into()),
        })
        .unwrap_err();
    assert!(refused.0.contains("unknown container config 'other'"));
}

#[test]
fn the_collector_debug_names_its_registry_only() {
    let rig = Rig::new();
    let shown = format!("{:?}", rig.use_case());
    assert!(shown.starts_with("GcOrphanedEnvironments"), "{shown}");
    assert!(shown.contains("registry"), "{shown}");
}

/// Review F4 (#2033): a state dir that has lost (or never wrote) its
/// `container` file is not thereby gone — the runtime's own listing may
/// still name a running container labelled with that environment id.
/// Its liveness comes from the listing: running is kept, exited is an
/// orphan with the listed container named (no create grace: the runtime
/// has already answered for it).
#[test]
fn a_dir_without_a_container_file_takes_its_liveness_from_the_listing() {
    let rig = Rig::new();
    *rig.host.dirs.lock().unwrap() = vec![
        dir("env-nofile-live", None),
        dir("env-nofile-dead", None),
        dir("env-nofile-unlisted", None),
    ];
    *rig.host.containers.lock().unwrap() = vec![
        container("env-nofile-live", true),
        container("env-nofile-dead", false),
    ];
    // The config's inspect would say gone (it reads the missing file);
    // the listing must win.
    let report = rig.dry_run();
    let kept: Vec<&str> = report
        .kept
        .iter()
        .map(|k| k.environment_id.as_str())
        .collect();
    assert_eq!(kept, ["env-nofile-live"], "{report:?}");
    assert!(
        report.kept[0]
            .reason
            .contains("container quecto-env-nofile-live is running"),
        "{report:?}"
    );
    let removable: Vec<(&str, Option<&str>)> = report
        .removable
        .iter()
        .map(|c| (c.environment_id.as_str(), c.container.as_deref()))
        .collect();
    assert_eq!(
        removable,
        [
            ("env-nofile-dead", Some("quecto-env-nofile-dead")),
            ("env-nofile-unlisted", None),
        ],
        "{report:?}"
    );
    assert!(
        report.removable[0]
            .reason
            .contains("container quecto-env-nofile-dead gone or exited"),
        "{report:?}"
    );
    // Nothing is ever removed on a dry run, least of all the live one.
    let report = rig
        .use_case()
        .execute(&GcRequest {
            dry_run: false,
            config: None,
        })
        .unwrap();
    assert_eq!(
        rig.host.removed.lock().unwrap().as_slice(),
        ["env-nofile-dead", "env-nofile-unlisted"],
        "{report:?}"
    );
    assert!(
        rig.host
            .containers
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.environment_id == "env-nofile-live"),
        "the running container survives the collection"
    );
}
