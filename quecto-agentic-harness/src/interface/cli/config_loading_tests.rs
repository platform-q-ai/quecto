use super::*;
use crate::application::configuration::dto::OverlayReport;
use std::path::PathBuf;

fn sources(overlay: Option<OverlayState>, legacy: bool) -> ConfigSources {
    ConfigSources {
        base: PathBuf::from("/home/u/.quecto/config.json"),
        explicit: false,
        overlay: overlay.map(|state| OverlayReport {
            path: PathBuf::from("/work/.quecto/config.json"),
            state,
        }),
        legacy_local: legacy.then(|| PathBuf::from("/work/config.json")),
    }
}

#[test]
fn diagnostics_name_the_untrusted_overlay_and_the_retired_file() {
    assert!(layer_diagnostics(&sources(Some(OverlayState::Applied), false)).is_empty());
    assert!(layer_diagnostics(&sources(None, false)).is_empty());
    let lines = layer_diagnostics(&sources(
        Some(OverlayState::Untrusted {
            fingerprint: "abc".into(),
        }),
        true,
    ));
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("/work/.quecto/config.json") && lines[0].contains("abc"));
    assert!(lines[0].contains("not applied") && lines[0].contains("quecto config trust"));
    assert!(lines[1].contains("/work/config.json") && lines[1].contains("no longer loaded"));
    assert!(lines[1].contains("/work/.quecto/config.json"));
}

#[test]
fn the_overlay_summary_reports_each_state() {
    assert_eq!(
        overlay_summary(&sources(Some(OverlayState::Applied), false)),
        "/work/.quecto/config.json (trusted)"
    );
    assert_eq!(
        overlay_summary(&sources(
            Some(OverlayState::Untrusted {
                fingerprint: "x".into()
            }),
            false
        )),
        "/work/.quecto/config.json (untrusted)"
    );
    assert_eq!(
        overlay_summary(&sources(Some(OverlayState::Absent), false)),
        "none"
    );
    assert_eq!(overlay_summary(&sources(None, false)), "none");
}

#[test]
fn loading_reports_a_missing_explicit_file_by_path() {
    let base = tempfile::TempDir::new().unwrap();
    let error = load_selected_config(
        crate::composition::configuration::build_configuration_handles,
        base.path(),
        &ConfigSelection::Explicit(PathBuf::from("/nowhere/config.json")),
        false,
        &HashMap::new(),
    )
    .err()
    .unwrap();
    assert_eq!(error, "config not found: /nowhere/config.json");
}
