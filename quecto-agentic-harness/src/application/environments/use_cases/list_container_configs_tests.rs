use std::sync::Arc;

use crate::application::environments::dto::{ContainerConfigEntry, ContainerConfigLayer};
use crate::application::environments::ports::{ContainerConfigRoster, ContainerConfigRosterReport};
use crate::application::environments::use_cases::ListContainerConfigs;

struct FixedRoster(Result<ContainerConfigRosterReport, String>);

impl ContainerConfigRoster for FixedRoster {
    fn roster(&self) -> Result<ContainerConfigRosterReport, String> {
        self.0.clone()
    }
}

fn entry(name: &str, default: bool, layer: ContainerConfigLayer) -> ContainerConfigEntry {
    ContainerConfigEntry {
        name: name.to_string(),
        default,
        layer,
        repository: Some(format!("https://example.test/{name}")),
        problem: None,
    }
}

fn names(inventory: &crate::application::environments::dto::ContainerConfigInventory) -> Vec<&str> {
    inventory.configs.iter().map(|e| e.name.as_str()).collect()
}

#[test]
fn the_default_comes_first_then_the_rest_by_name() {
    let query = ListContainerConfigs::new(Arc::new(FixedRoster(Ok(ContainerConfigRosterReport {
        configs: vec![
            entry("zeta", false, ContainerConfigLayer::Global),
            entry("r", true, ContainerConfigLayer::Overlay),
            entry("alpha", false, ContainerConfigLayer::Global),
        ],
        overlay_withheld: false,
        diagnostics: vec![],
    }))));
    let inventory = query.execute().unwrap();
    assert_eq!(names(&inventory), ["r", "alpha", "zeta"]);
    assert_eq!(
        inventory.default_entry().map(|e| e.name.as_str()),
        Some("r")
    );
    assert_eq!(inventory.configs[0].layer, ContainerConfigLayer::Overlay);
    assert_eq!(
        inventory.configs[0].repository.as_deref(),
        Some("https://example.test/r")
    );
    assert!(!inventory.overlay_withheld);
}

#[test]
fn a_withheld_overlay_marks_no_entry_default_and_keeps_the_diagnostics() {
    let query = ListContainerConfigs::new(Arc::new(FixedRoster(Ok(ContainerConfigRosterReport {
        configs: vec![
            entry("default", true, ContainerConfigLayer::Global),
            entry("alternate", false, ContainerConfigLayer::Global),
        ],
        overlay_withheld: true,
        diagnostics: vec!["repo-local config overlay /c/.quecto/config.json is not trusted".into()],
    }))));
    let inventory = query.execute().unwrap();
    assert!(inventory.default_entry().is_none());
    assert!(inventory.configs.iter().all(|e| !e.default));
    // Without a default the order is by name alone.
    assert_eq!(names(&inventory), ["alternate", "default"]);
    assert!(inventory.overlay_withheld);
    assert_eq!(inventory.diagnostics.len(), 1);
}

#[test]
fn an_empty_set_lists_nothing() {
    let query = ListContainerConfigs::new(Arc::new(FixedRoster(Ok(
        ContainerConfigRosterReport::default(),
    ))));
    let inventory = query.execute().unwrap();
    assert!(inventory.configs.is_empty());
    assert!(inventory.default_entry().is_none());
}

#[test]
fn the_ports_failure_is_the_result() {
    let query = ListContainerConfigs::new(Arc::new(FixedRoster(Err(
        "container spawn requires --config so container_configs can be loaded".into(),
    ))));
    assert_eq!(
        query.execute().unwrap_err(),
        "container spawn requires --config so container_configs can be loaded"
    );
}

#[test]
fn layer_wire_words_are_stable() {
    assert_eq!(ContainerConfigLayer::Overlay.as_str(), "overlay");
    assert_eq!(ContainerConfigLayer::Global.as_str(), "global");
}

#[test]
fn an_entry_a_launch_would_refuse_is_never_default_and_is_diagnosed() {
    let mut broken = entry("broken", true, ContainerConfigLayer::Overlay);
    broken.problem = Some("missing cleanup argv".into());
    let query = ListContainerConfigs::new(Arc::new(FixedRoster(Ok(ContainerConfigRosterReport {
        configs: vec![broken, entry("alpha", false, ContainerConfigLayer::Global)],
        overlay_withheld: false,
        diagnostics: vec![],
    }))));
    let inventory = query.execute().unwrap();
    assert!(inventory.default_entry().is_none());
    assert_eq!(names(&inventory), ["alpha", "broken"]);
    assert_eq!(
        inventory.diagnostics,
        ["container config 'broken' cannot launch as configured: missing cleanup argv"]
    );
    assert_eq!(
        inventory.configs[1].problem.as_deref(),
        Some("missing cleanup argv")
    );
}

#[test]
fn more_than_one_labelled_default_marks_none_and_is_diagnosed() {
    let query = ListContainerConfigs::new(Arc::new(FixedRoster(Ok(ContainerConfigRosterReport {
        configs: vec![
            entry("b", true, ContainerConfigLayer::Global),
            entry("a", true, ContainerConfigLayer::Global),
        ],
        overlay_withheld: false,
        diagnostics: vec![],
    }))));
    let inventory = query.execute().unwrap();
    assert!(inventory.configs.iter().all(|e| !e.default));
    assert_eq!(names(&inventory), ["a", "b"]);
    assert_eq!(inventory.diagnostics.len(), 1);
    assert!(
        inventory.diagnostics[0].contains("(a, b)"),
        "{:?}",
        inventory.diagnostics
    );
}
