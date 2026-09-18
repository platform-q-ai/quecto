use std::sync::Arc;

use super::{ROSTER_LINE_MAX_CHARS, ROSTER_PREFIX, format_roster_line, roster_line};
use crate::application::environments::dto::{
    ContainerConfigEntry, ContainerConfigInventory, ContainerConfigLayer,
};
use crate::application::environments::ports::{ContainerConfigRoster, ContainerConfigRosterReport};
use crate::application::environments::use_cases::ListContainerConfigs;

fn entry(name: &str, default: bool, layer: ContainerConfigLayer) -> ContainerConfigEntry {
    ContainerConfigEntry {
        name: name.to_string(),
        default,
        layer,
        repository: None,
        problem: None,
        joinable: false,
    }
}

fn inventory(configs: Vec<ContainerConfigEntry>, withheld: bool) -> ContainerConfigInventory {
    ContainerConfigInventory {
        configs,
        overlay_withheld: withheld,
        diagnostics: vec![],
    }
}

#[test]
fn the_budget_is_the_documented_120_characters() {
    assert_eq!(ROSTER_LINE_MAX_CHARS, 120);
}

#[test]
fn names_the_default_first_with_its_layer() {
    let line = format_roster_line(&inventory(
        vec![
            entry("r", true, ContainerConfigLayer::Overlay),
            entry("alternate", false, ContainerConfigLayer::Global),
            entry("default", false, ContainerConfigLayer::Global),
        ],
        false,
    ));
    assert_eq!(
        line,
        "Available container configs: r (default, repo-bound), alternate (global), default (global)."
    );
}

#[test]
fn a_withheld_overlay_is_named_and_the_note_survives_truncation() {
    let mut configs: Vec<ContainerConfigEntry> = (0..20)
        .map(|i| {
            entry(
                &format!("config-{i:02}"),
                false,
                ContainerConfigLayer::Global,
            )
        })
        .collect();
    configs.push(entry("alt", false, ContainerConfigLayer::Global));
    let line = format_roster_line(&inventory(configs, true));
    assert!(line.chars().count() <= ROSTER_LINE_MAX_CHARS, "{line}");
    assert!(
        line.ends_with(" more (repo overlay untrusted — run quecto config trust)."),
        "{line}"
    );
    assert!(line.starts_with(&format!("{ROSTER_PREFIX}config-00 (global)")));
}

#[test]
fn a_long_roster_folds_the_tail_into_a_count() {
    let configs: Vec<ContainerConfigEntry> = (0..12)
        .map(|i| {
            entry(
                &format!("many-{i:02}"),
                i == 0,
                ContainerConfigLayer::Global,
            )
        })
        .collect();
    let line = format_roster_line(&inventory(configs, false));
    assert!(line.chars().count() <= ROSTER_LINE_MAX_CHARS, "{line}");
    assert!(line.contains("many-00 (default, global)"), "{line}");
    let more: usize = line
        .split(", +")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|n| n.parse().ok())
        .expect("a +N more suffix");
    let shown = line.matches("many-").count();
    assert_eq!(shown + more, 12, "{line}");
    assert!(line.ends_with(" more."), "{line}");
}

#[test]
fn one_entry_alone_over_budget_is_cut_within_the_budget() {
    let long = "x".repeat(200);
    let line = format_roster_line(&inventory(
        vec![entry(&long, true, ContainerConfigLayer::Global)],
        true,
    ));
    assert!(line.chars().count() <= ROSTER_LINE_MAX_CHARS, "{line}");
    assert!(
        line.starts_with("Available container configs: xxxx"),
        "{line}"
    );
    assert!(
        line.ends_with("… (repo overlay untrusted — run quecto config trust)."),
        "{line}"
    );
}

#[test]
fn an_empty_set_says_so() {
    assert_eq!(
        format_roster_line(&inventory(vec![], false)),
        "Available container configs: none configured."
    );
    assert_eq!(
        format_roster_line(&inventory(vec![], true)),
        "Available container configs: none configured (repo overlay untrusted — run quecto config trust)."
    );
}

struct Failing;

impl ContainerConfigRoster for Failing {
    fn roster(&self) -> Result<ContainerConfigRosterReport, String> {
        Err(format!("line one\nline two {}", "f".repeat(300)))
    }

    fn revision(&self) -> String {
        "r".into()
    }
}

#[test]
fn an_unreadable_configuration_still_yields_a_bounded_line() {
    let query = ListContainerConfigs::new(Arc::new(Failing));
    let line = roster_line(Some(&query)).unwrap();
    assert!(line.starts_with("Available container configs: none readable (line one line two fff"));
    assert!(!line.contains('\n'));
    assert!(line.chars().count() <= ROSTER_LINE_MAX_CHARS, "{line}");
    assert!(line.ends_with("…."), "{line}");
    assert!(roster_line(None).is_none());
}

#[test]
fn the_hidden_count_survives_when_the_first_entry_alone_overflows() {
    let line = format_roster_line(&inventory(
        vec![
            entry(&"a".repeat(100), false, ContainerConfigLayer::Global),
            entry("b", false, ContainerConfigLayer::Global),
            entry("c", false, ContainerConfigLayer::Global),
        ],
        true,
    ));
    assert!(line.chars().count() <= ROSTER_LINE_MAX_CHARS, "{line}");
    assert!(
        line.ends_with("…, +2 more (repo overlay untrusted — run quecto config trust)."),
        "{line}"
    );
}

#[test]
fn names_with_newlines_keep_the_roster_on_one_line() {
    let line = format_roster_line(&inventory(
        vec![entry("odd\nname", true, ContainerConfigLayer::Global)],
        false,
    ));
    assert_eq!(
        line,
        "Available container configs: odd name (default, global)."
    );
}

/// A roster whose revision the test moves; counts how often the listing
/// is actually read.
struct Counting {
    revision: std::sync::Mutex<String>,
    reads: std::sync::atomic::AtomicUsize,
}

impl ContainerConfigRoster for Counting {
    fn roster(&self) -> Result<ContainerConfigRosterReport, String> {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(ContainerConfigRosterReport {
            configs: vec![entry(
                &format!("rev-{}", self.revision.lock().unwrap()),
                true,
                ContainerConfigLayer::Global,
            )],
            overlay_withheld: false,
            diagnostics: vec![],
        })
    }

    fn revision(&self) -> String {
        self.revision.lock().unwrap().clone()
    }
}

#[test]
fn the_spawn_definition_reads_the_configuration_only_when_the_revision_changes() {
    use crate::application::tools::ports::Tool;
    let counting = Arc::new(Counting {
        revision: std::sync::Mutex::new("1".into()),
        reads: std::sync::atomic::AtomicUsize::new(0),
    });
    let tool = crate::infrastructure::tools::spawn::SpawnTool::new(vec![])
        .with_container_config_roster(Some(Arc::new(ListContainerConfigs::new(counting.clone()))));
    for _ in 0..5 {
        let description = tool.definition().description;
        assert!(
            description.contains("rev-1 (default, global)"),
            "{description}"
        );
    }
    assert_eq!(counting.reads.load(std::sync::atomic::Ordering::SeqCst), 1);
    *counting.revision.lock().unwrap() = "2".into();
    let description = tool.definition().description;
    assert!(
        description.contains("rev-2 (default, global)"),
        "{description}"
    );
    assert!(!description.contains("rev-1"), "{description}");
    assert_eq!(counting.reads.load(std::sync::atomic::Ordering::SeqCst), 2);
}
