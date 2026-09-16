use super::*;
use std::path::PathBuf;
use std::sync::Mutex;

/// Records every probed path and answers with a fixed presence.
struct FakeProbe {
    presence: LocalConfigPresence,
    probed: Mutex<Vec<PathBuf>>,
}

impl FakeProbe {
    fn answering(presence: LocalConfigPresence) -> Arc<Self> {
        Arc::new(Self {
            presence,
            probed: Mutex::new(Vec::new()),
        })
    }

    fn probed(&self) -> Vec<PathBuf> {
        self.probed.lock().unwrap().clone()
    }
}

impl LocalConfigProbe for FakeProbe {
    fn probe(&self, path: &Path) -> LocalConfigPresence {
        self.probed.lock().unwrap().push(path.to_path_buf());
        self.presence.clone()
    }
}

fn request(explicit: Option<&str>, cwd: Option<&str>) -> ConfigSelectionRequest {
    ConfigSelectionRequest {
        explicit: explicit.map(PathBuf::from),
        working_directory: cwd.map(PathBuf::from),
        global: PathBuf::from("/home/u/.quecto/config.json"),
    }
}

#[test]
fn explicit_override_wins_without_probing_the_working_directory() {
    let probe = FakeProbe::answering(LocalConfigPresence::RegularFile);
    let selected = SelectConfig::new(probe.clone())
        .execute(request(Some("/explicit.json"), Some("/work")))
        .unwrap();
    assert_eq!(selected, ConfigSelection::Explicit("/explicit.json".into()));
    assert!(
        probe.probed().is_empty(),
        "explicit selection must not touch the cwd"
    );
}

#[test]
fn present_local_file_is_selected_and_only_the_cwd_itself_is_probed() {
    let probe = FakeProbe::answering(LocalConfigPresence::RegularFile);
    let selected = SelectConfig::new(probe.clone())
        .execute(request(None, Some("/work/nested")))
        .unwrap();
    assert_eq!(
        selected,
        ConfigSelection::WorkingDirectory("/work/nested/config.json".into())
    );
    assert_eq!(
        probe.probed(),
        vec![PathBuf::from("/work/nested/config.json")]
    );
}

#[test]
fn absent_local_file_falls_back_to_global() {
    let probe = FakeProbe::answering(LocalConfigPresence::Absent);
    let selected = SelectConfig::new(probe)
        .execute(request(None, Some("/work")))
        .unwrap();
    assert_eq!(
        selected,
        ConfigSelection::Global("/home/u/.quecto/config.json".into())
    );
}

#[test]
fn unknown_working_directory_falls_back_to_global_without_probing() {
    let probe = FakeProbe::answering(LocalConfigPresence::RegularFile);
    let selected = SelectConfig::new(probe.clone())
        .execute(request(None, None))
        .unwrap();
    assert_eq!(
        selected,
        ConfigSelection::Global("/home/u/.quecto/config.json".into())
    );
    assert!(probe.probed().is_empty());
}

#[test]
fn non_regular_local_entry_is_an_error_naming_the_path() {
    let probe = FakeProbe::answering(LocalConfigPresence::NotRegularFile);
    let error = SelectConfig::new(probe)
        .execute(request(None, Some("/work")))
        .unwrap_err();
    assert_eq!(
        error,
        ConfigSelectionError {
            path: "/work/config.json".into(),
            rejection: LocalConfigRejection::NotRegularFile,
        }
    );
}

#[test]
fn unreadable_local_entry_is_an_error_not_a_fallback() {
    let probe = FakeProbe::answering(LocalConfigPresence::Unreadable("permission denied".into()));
    let error = SelectConfig::new(probe)
        .execute(request(None, Some("/work")))
        .unwrap_err();
    assert_eq!(
        error,
        ConfigSelectionError {
            path: "/work/config.json".into(),
            rejection: LocalConfigRejection::Unreadable("permission denied".into()),
        }
    );
}

#[test]
fn debug_does_not_expose_the_probe() {
    let probe = FakeProbe::answering(LocalConfigPresence::Absent);
    assert_eq!(
        format!("{:?}", SelectConfig::new(probe)),
        "SelectConfig { .. }"
    );
}
