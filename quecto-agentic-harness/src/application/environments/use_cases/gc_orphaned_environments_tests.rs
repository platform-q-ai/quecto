use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::super::dto::{
    ContainerRuntimeTarget, DiagnosableContainerConfig, EnvironmentLiveness, EnvironmentStateDir,
    GcRemoval, GcRequest, RuntimeContainer,
};
use super::super::ports::{ContainerConfigLookup, ContainerRuntimeInventory, EnvironmentProcess};
use super::{GcOrphanedEnvironments, implied_state_root};
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

#[derive(Default)]
struct FakeInventory {
    containers: Mutex<Vec<RuntimeContainer>>,
    dirs: Mutex<Vec<EnvironmentStateDir>>,
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
    fn remove(
        &self,
        _config: &DiagnosableContainerConfig,
        environment_id: &str,
    ) -> Result<(), String> {
        if environment_id == "env-broken" {
            return Err("cleanup exited 1".into());
        }
        self.removed.lock().unwrap().push(environment_id.into());
        self.containers
            .lock()
            .unwrap()
            .retain(|c| c.environment_id != environment_id);
        if !self.stubborn.iter().any(|id| id == environment_id) {
            self.dirs
                .lock()
                .unwrap()
                .retain(|d| d.environment_id != environment_id);
        }
        Ok(())
    }
}

#[derive(Default)]
struct FakeProcess {
    liveness: Mutex<Vec<(String, EnvironmentLiveness)>>,
    cleaned: Mutex<Vec<String>>,
    dirs: Option<Arc<FakeInventory>>,
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
        if let Some(inventory) = &self.dirs {
            inventory
                .dirs
                .lock()
                .unwrap()
                .retain(|d| d.environment_id != record.environment_id);
            inventory
                .containers
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
    inventory: Arc<FakeInventory>,
    process: Arc<FakeProcess>,
    lookup: Result<DiagnosableContainerConfig, String>,
}

impl Rig {
    fn new() -> Self {
        let inventory = Arc::new(FakeInventory::default());
        Self {
            registry: EnvironmentRegistry::new(),
            process: Arc::new(FakeProcess {
                dirs: Some(inventory.clone()),
                ..Default::default()
            }),
            inventory,
            lookup: Ok(config()),
        }
    }
    fn use_case(&self) -> GcOrphanedEnvironments {
        GcOrphanedEnvironments::new(
            self.registry.clone(),
            Arc::new(FakeLookup(self.lookup.clone())),
            self.inventory.clone(),
            self.process.clone(),
        )
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
fn orphans_are_exited_or_unknown_containers_with_no_record_or_a_stopped_one() {
    let rig = Rig::new();
    rig.registry
        .commit(record("C1", "env-live", EnvironmentStatus::Running));
    rig.registry
        .commit(record("C2", "env-stopped", EnvironmentStatus::Stopped));
    rig.registry
        .commit(record("C3", "env-believed", EnvironmentStatus::Retained));
    *rig.process.liveness.lock().unwrap() = vec![("C3".into(), EnvironmentLiveness::Gone)];
    *rig.inventory.dirs.lock().unwrap() = vec![
        dir("env-live", Some("quecto-env-live")),
        dir("env-stopped", Some("quecto-env-stopped")),
        dir("env-believed", Some("quecto-env-believed")),
        dir("env-orphan", Some("quecto-env-orphan")),
        dir("env-nocontainer", None),
        dir("env-running-unknown", Some("quecto-env-running-unknown")),
    ];
    *rig.inventory.containers.lock().unwrap() = vec![
        container("env-live", true),
        container("env-stopped", false),
        container("env-orphan", false),
        container("env-running-unknown", true),
        container("env-ghost", false),
    ];
    let report = rig
        .use_case()
        .execute(&GcRequest {
            dry_run: true,
            config: None,
            state_roots: vec![PathBuf::from("/extra")],
        })
        .unwrap();
    assert!(report.dry_run);
    assert_eq!(report.config, "box");
    assert_eq!(
        report.state_roots,
        vec![PathBuf::from("/extra"), PathBuf::from("/s")]
    );
    let removable: Vec<(&str, &GcRemoval)> = report
        .removable
        .iter()
        .map(|c| (c.environment_id.as_str(), &c.removal))
        .collect();
    assert_eq!(
        removable,
        vec![
            (
                "env-nocontainer",
                &GcRemoval::ConfiguredCleanup {
                    config: "box".into()
                }
            ),
            (
                "env-orphan",
                &GcRemoval::ConfiguredCleanup {
                    config: "box".into()
                }
            ),
            (
                "env-stopped",
                &GcRemoval::RetainedCleanup {
                    environment_ref: "C2".into()
                }
            ),
            (
                "env-ghost",
                &GcRemoval::ConfiguredCleanup {
                    config: "box".into()
                }
            ),
        ]
    );
    let ghost = &report.removable[3];
    assert_eq!(ghost.state_dir, None);
    assert_eq!(ghost.container.as_deref(), Some("quecto-env-ghost"));
    let kept: Vec<&str> = report
        .kept
        .iter()
        .map(|k| k.environment_id.as_str())
        .collect();
    assert_eq!(kept, ["env-believed", "env-live", "env-running-unknown"]);
    let believed = &report.kept[0];
    assert!(
        believed.reason.contains("recorded C3 as retained"),
        "{believed:?}"
    );
    assert!(
        believed.reason.contains("inspect reports it gone"),
        "{believed:?}"
    );
    assert!(
        believed.reason.contains("quecto container kill"),
        "{believed:?}"
    );
    assert!(report.removed.is_empty() && report.errors.is_empty());
    assert!(rig.inventory.removed.lock().unwrap().is_empty());
    assert!(rig.process.cleaned.lock().unwrap().is_empty());
}

#[test]
fn a_real_run_removes_through_the_retained_or_configured_cleanup_and_reports_leftovers() {
    let rig = Rig::new();
    rig.registry
        .commit(record("C2", "env-stopped", EnvironmentStatus::Stopped));
    let mut no_cleanup = record("C4", "env-bare", EnvironmentStatus::Stopped);
    no_cleanup.retained_cleanup_argv.clear();
    rig.registry.commit(no_cleanup);
    *rig.inventory.dirs.lock().unwrap() = vec![
        dir("env-stopped", Some("quecto-env-stopped")),
        dir("env-bare", None),
        dir("env-orphan", Some("quecto-env-orphan")),
        dir("env-broken", None),
        dir("env-stubborn", None),
    ];
    *rig.inventory.containers.lock().unwrap() = vec![container("env-orphan", false)];
    let rig = Rig {
        inventory: Arc::new(FakeInventory {
            containers: Mutex::new(rig.inventory.containers.lock().unwrap().clone()),
            dirs: Mutex::new(rig.inventory.dirs.lock().unwrap().clone()),
            stubborn: vec!["env-stubborn".into()],
            ..Default::default()
        }),
        ..rig
    };
    let process = Arc::new(FakeProcess {
        dirs: Some(rig.inventory.clone()),
        ..Default::default()
    });
    let rig = Rig { process, ..rig };
    let report = rig.use_case().execute(&GcRequest::default()).unwrap();
    let removed: Vec<&str> = report
        .removed
        .iter()
        .map(|c| c.environment_id.as_str())
        .collect();
    assert_eq!(removed, ["env-bare", "env-orphan", "env-stopped"]);
    assert_eq!(rig.process.cleaned.lock().unwrap().as_slice(), ["C2"]);
    assert_eq!(
        rig.inventory.removed.lock().unwrap().as_slice(),
        ["env-bare", "env-orphan", "env-stubborn"]
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
fn a_running_record_with_a_live_container_is_kept_whatever_its_inspect_says() {
    let rig = Rig::new();
    rig.registry
        .commit(record("C1", "env-live", EnvironmentStatus::CleanupFailed));
    *rig.inventory.dirs.lock().unwrap() = vec![dir("env-live", Some("quecto-env-live"))];
    *rig.inventory.containers.lock().unwrap() = vec![container("env-live", true)];
    let report = rig.use_case().execute(&GcRequest::default()).unwrap();
    assert!(report.removable.is_empty());
    assert_eq!(
        report.kept[0].reason,
        "container quecto-env-live is running"
    );
}

#[test]
fn an_unavailable_inventory_judges_nothing_and_an_unreadable_root_is_reported() {
    let mut rig = Rig::new();
    rig.inventory = Arc::new(FakeInventory {
        fail_list: true,
        ..Default::default()
    });
    let report = rig.use_case().execute(&GcRequest::default()).unwrap();
    assert!(report.removable.is_empty() && report.kept.is_empty());
    assert_eq!(report.errors.len(), 1);
    assert!(report.errors[0].contains("inspect --list unsupported"));

    let rig = Rig::new();
    let report = rig
        .use_case()
        .execute(&GcRequest {
            dry_run: true,
            config: None,
            state_roots: vec![PathBuf::from("/unreadable")],
        })
        .unwrap();
    assert_eq!(report.errors.len(), 1);
    assert!(report.errors[0].contains("state root /unreadable not scanned: permission denied"));
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
            state_roots: vec![],
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
