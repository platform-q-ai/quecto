use super::*;
use crate::application::configuration::dto::{OverlayOrigin, OverlayReport};
use std::path::PathBuf;

fn sources(overlay: Option<OverlayState>, legacy: bool) -> ConfigSources {
    ConfigSources {
        base: PathBuf::from("/home/u/.quecto/config.json"),
        explicit: false,
        overlay: overlay.map(|state| OverlayReport {
            path: PathBuf::from("/work/.quecto/config.json"),
            state,
            origin: Default::default(),
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
            problem: None,
            sections: Vec::new(),
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
fn diagnostics_carry_the_check_failure_of_an_untrusted_overlay_and_a_refusal() {
    let lines = layer_diagnostics(&sources(
        Some(OverlayState::Untrusted {
            fingerprint: "abc".into(),
            problem: Some("`providers` is global-only".into()),
            sections: Vec::new(),
        }),
        false,
    ));
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("would refuse it"), "{}", lines[0]);
    assert!(
        lines[0].contains("`providers` is global-only"),
        "{}",
        lines[0]
    );
    assert!(
        !lines[0].contains("then run `quecto config trust`"),
        "no remedy that would fail: {}",
        lines[0]
    );
    let lines = layer_diagnostics(&sources(
        Some(OverlayState::Refused {
            reason: "it is a symbolic link".into(),
        }),
        false,
    ));
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].contains("was not applied: it is a symbolic link"),
        "{}",
        lines[0]
    );
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
                fingerprint: "x".into(),
                problem: None,
                sections: Vec::new(),
            }),
            false
        )),
        "/work/.quecto/config.json (untrusted)"
    );
    assert_eq!(
        overlay_summary(&sources(
            Some(OverlayState::Refused {
                reason: "it is a symbolic link".into()
            }),
            false
        )),
        "/work/.quecto/config.json (refused)"
    );
    assert_eq!(
        overlay_summary(&sources(Some(OverlayState::Absent), false)),
        "none"
    );
    assert_eq!(overlay_summary(&sources(None, false)), "none");
    let explicit = ConfigSources {
        explicit: true,
        ..sources(None, false)
    };
    assert_eq!(
        overlay_summary(&explicit),
        "none (--config replaces both layers)"
    );
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
        false,
    )
    .err()
    .unwrap();
    assert_eq!(error, "config not found: /nowhere/config.json");
}

fn with_origin(state: OverlayState, origin: OverlayOrigin) -> ConfigSources {
    let mut sources = sources(Some(state), false);
    sources.overlay.as_mut().unwrap().origin = origin;
    sources
}

#[test]
fn the_overlay_summary_says_a_shared_overlay_comes_from_the_main_worktree() {
    let shared = |state| overlay_summary(&with_origin(state, OverlayOrigin::MainWorktree));
    assert_eq!(
        shared(OverlayState::Applied),
        "/work/.quecto/config.json (trusted, from the main worktree)"
    );
    assert_eq!(
        shared(OverlayState::Untrusted {
            fingerprint: "abc".into(),
            problem: None,
            sections: Vec::new(),
        }),
        "/work/.quecto/config.json (untrusted, from the main worktree)"
    );
    assert_eq!(
        shared(OverlayState::Refused {
            reason: "link".into()
        }),
        "/work/.quecto/config.json (refused, from the main worktree)"
    );
    assert_eq!(shared(OverlayState::Absent), "none");
}

#[test]
fn an_unresolved_main_worktree_is_a_diagnostic_naming_why_and_the_own_location() {
    let unresolved = OverlayOrigin::MainWorktreeUnresolved {
        reason: "gitdir: /gone".into(),
    };
    let sources = with_origin(OverlayState::Absent, unresolved.clone());
    assert_eq!(overlay_summary(&sources), "none");
    let lines = layer_diagnostics(&sources);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("linked git worktree") && lines[0].contains("gitdir: /gone"));
    assert!(
        lines[0].contains("/work/.quecto/config.json"),
        "{}",
        lines[0]
    );
    // With the own overlay untrusted, both lines are said.
    let untrusted = OverlayState::Untrusted {
        fingerprint: "abc".into(),
        problem: None,
        sections: Vec::new(),
    };
    assert_eq!(
        layer_diagnostics(&with_origin(untrusted, unresolved)).len(),
        2
    );
}
