//! A linked git worktree with no overlay of its own shares its main
//! worktree's, under that file's own trust record: the composed graph over
//! real git worktrees, the real document store and the real trust record.

use super::*;
use crate::application::configuration::dto::{
    ConfigLayer, ConfigPatch, ConfigPatchError, ConfigSelectionRequest, EffectiveConfig,
    OverlayOrigin, OverlayState, OverlayTrustRequest,
};
use crate::infrastructure::config::main_worktree::tests::{Layout, layout};
use crate::interface::cli::controllers::configuration_handles::ConfigurationHandles;
use std::path::PathBuf;
use tempfile::TempDir;

const MAIN_MODEL: &str = r#"{"agents":{"defaults":{"model":"from-main"}}}"#;
const OWN_MODEL: &str = r#"{"agents":{"defaults":{"model":"from-linked"}}}"#;

struct Rig {
    base: TempDir,
    layout: Layout,
    handles: ConfigurationHandles,
}

impl Rig {
    fn new() -> Self {
        let base = TempDir::new().unwrap();
        std::fs::write(
            base.path().join("config.json"),
            r#"{"agents":{"defaults":{"model":"global"}}}"#,
        )
        .unwrap();
        let handles = build_configuration_handles(&ConfigurationEnvironment {
            base_dir: base.path().to_path_buf(),
            prompt_for_trust: false,
        });
        Self {
            base,
            layout: layout(),
            handles,
        }
    }

    fn write_overlay(&self, worktree: &Path, content: &str) -> PathBuf {
        let overlay = worktree.join(".quecto/config.json");
        std::fs::create_dir_all(overlay.parent().unwrap()).unwrap();
        std::fs::write(&overlay, content).unwrap();
        overlay
    }

    fn trust(&self, overlay: &Path) {
        self.handles
            .trust
            .execute(OverlayTrustRequest {
                path: overlay.to_path_buf(),
            })
            .unwrap();
    }

    fn select(&self, cwd: &Path) -> ConfigSelection {
        self.handles.select.execute(ConfigSelectionRequest {
            explicit: None,
            working_directory: Some(cwd.to_path_buf()),
            global: self.base.path().join("config.json"),
        })
    }

    fn resolve(&self, cwd: &Path) -> EffectiveConfig {
        self.handles.resolve.execute(&self.select(cwd)).unwrap()
    }

    fn trust_record(&self) -> Vec<u8> {
        std::fs::read(self.base.path().join(TRUST_RECORD_FILE_NAME)).unwrap_or_default()
    }
}

fn model(effective: &EffectiveConfig) -> &str {
    effective.document["agents"]["defaults"]["model"]
        .as_str()
        .unwrap()
}

#[test]
fn a_linked_worktree_applies_the_main_worktrees_trusted_overlay_and_records_nothing() {
    let rig = Rig::new();
    let main_overlay = rig.write_overlay(&rig.layout.main, MAIN_MODEL);
    rig.trust(&main_overlay);
    let record = rig.trust_record();

    let effective = rig.resolve(&rig.layout.linked);
    assert_eq!(model(&effective), "from-main");
    let report = effective.sources.overlay.as_ref().unwrap();
    assert_eq!(report.path, main_overlay);
    assert_eq!(report.state, OverlayState::Applied);
    assert_eq!(report.origin, OverlayOrigin::MainWorktree);
    assert!(effective.sources.diagnostics().is_empty());
    assert_eq!(rig.trust_record(), record, "no trust is recorded");
    assert!(
        !rig.layout.linked.join(".quecto").exists(),
        "nothing is copied"
    );

    // The reload watches the shared file and the record it is judged by.
    let watched = watched_config_sources(rig.base.path(), &rig.select(&rig.layout.linked));
    assert_eq!(watched.len(), 3);
}

#[test]
fn a_linked_worktree_withholds_the_main_worktrees_untrusted_overlay() {
    let rig = Rig::new();
    let main_overlay = rig.write_overlay(&rig.layout.main, MAIN_MODEL);

    let effective = rig.resolve(&rig.layout.linked);
    assert_eq!(model(&effective), "global");
    let report = effective.sources.overlay.as_ref().unwrap();
    assert_eq!(report.path, main_overlay);
    assert!(matches!(report.state, OverlayState::Untrusted { .. }));
    assert_eq!(report.origin, OverlayOrigin::MainWorktree);

    // A hand edit after approval revokes it for both worktrees at once.
    rig.trust(&main_overlay);
    assert_eq!(model(&rig.resolve(&rig.layout.linked)), "from-main");
    std::fs::write(
        &main_overlay,
        r#"{"agents":{"defaults":{"model":"edited"}}}"#,
    )
    .unwrap();
    assert_eq!(model(&rig.resolve(&rig.layout.linked)), "global");
    assert_eq!(model(&rig.resolve(&rig.layout.main)), "global");
}

#[test]
fn a_linked_worktree_with_its_own_overlay_keeps_its_own_file_and_trust() {
    let rig = Rig::new();
    let main_overlay = rig.write_overlay(&rig.layout.main, MAIN_MODEL);
    rig.trust(&main_overlay);
    let own = rig.write_overlay(&rig.layout.linked, OWN_MODEL);

    let untrusted = rig.resolve(&rig.layout.linked);
    assert_eq!(
        model(&untrusted),
        "global",
        "the main's trust is not borrowed"
    );
    let report = untrusted.sources.overlay.as_ref().unwrap();
    assert_eq!(report.path, own);
    assert_eq!(report.origin, OverlayOrigin::WorkingDirectory);

    rig.trust(&own);
    assert_eq!(model(&rig.resolve(&rig.layout.linked)), "from-linked");
}

#[test]
fn a_broken_gitdir_shares_nothing_and_says_why() {
    let rig = Rig::new();
    let main_overlay = rig.write_overlay(&rig.layout.main, MAIN_MODEL);
    rig.trust(&main_overlay);
    std::fs::write(rig.layout.linked.join(".git"), "gitdir: /nonexistent/x\n").unwrap();

    let effective = rig.resolve(&rig.layout.linked);
    assert_eq!(model(&effective), "global");
    let report = effective.sources.overlay.as_ref().unwrap();
    assert_eq!(report.path, rig.layout.linked.join(".quecto/config.json"));
    assert_eq!(report.state, OverlayState::Absent);
    assert!(matches!(
        report.origin,
        OverlayOrigin::MainWorktreeUnresolved { .. }
    ));
    let diagnostics = effective.sources.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(diagnostics[0].contains("/nonexistent/x"), "{diagnostics:?}");
}

#[test]
fn a_symlinked_main_overlay_is_refused_in_the_linked_worktree_too() {
    let rig = Rig::new();
    let elsewhere = rig.layout.tmp.path().join("elsewhere.json");
    std::fs::write(&elsewhere, MAIN_MODEL).unwrap();
    let main_overlay = rig.layout.main.join(".quecto/config.json");
    std::fs::create_dir_all(main_overlay.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &main_overlay).unwrap();

    let effective = rig.resolve(&rig.layout.linked);
    assert_eq!(model(&effective), "global");
    let report = effective.sources.overlay.as_ref().unwrap();
    assert_eq!(report.path, main_overlay);
    assert!(matches!(report.state, OverlayState::Refused { .. }));
    assert_eq!(report.origin, OverlayOrigin::MainWorktree);
}

#[test]
fn a_shared_overlay_is_never_written_from_the_linked_worktree() {
    let rig = Rig::new();
    let main_overlay = rig.write_overlay(&rig.layout.main, MAIN_MODEL);
    rig.trust(&main_overlay);
    let record = rig.trust_record();

    let refused = rig
        .handles
        .patch
        .execute(ConfigPatch {
            selection: rig.select(&rig.layout.linked),
            layer: ConfigLayer::Overlay,
            key_path: "agents.defaults.model".into(),
            value: serde_json::json!("changed"),
        })
        .unwrap_err();
    assert_eq!(
        refused,
        ConfigPatchError::SharedOverlay {
            path: main_overlay.clone()
        }
    );
    assert_eq!(std::fs::read_to_string(&main_overlay).unwrap(), MAIN_MODEL);
    assert_eq!(rig.trust_record(), record);
}

/// An unresolved main worktree fails closed to the worktree's own
/// location, and that location is the worktree's to write: a patch there
/// creates its own overlay (recording trust for it), never the main's.
#[test]
fn a_patch_from_an_unresolved_worktree_writes_its_own_overlay() {
    let rig = Rig::new();
    let main_overlay = rig.write_overlay(&rig.layout.main, MAIN_MODEL);
    rig.trust(&main_overlay);
    std::fs::write(rig.layout.linked.join(".git"), "gitdir: /nonexistent/x\n").unwrap();

    let receipt = rig
        .handles
        .patch
        .execute(ConfigPatch {
            selection: rig.select(&rig.layout.linked),
            layer: ConfigLayer::Overlay,
            key_path: "agents.defaults.model".into(),
            value: serde_json::json!("own"),
        })
        .unwrap();
    let own = rig.layout.linked.join(".quecto/config.json");
    assert_eq!(receipt.path, own);
    assert_eq!(std::fs::read_to_string(&main_overlay).unwrap(), MAIN_MODEL);
    assert_eq!(model(&rig.resolve(&rig.layout.linked)), "own");
}

/// The refusal says how to give the worktree an overlay of its own.
#[test]
fn the_shared_overlay_refusal_says_how_to_make_an_own_overlay() {
    let message = ConfigPatchError::SharedOverlay {
        path: PathBuf::from("/main/.quecto/config.json"),
    }
    .to_string();
    for needle in [
        "/main/.quecto/config.json",
        "shared with this linked worktree",
        "replaces the main worktree's entirely",
        "quecto config trust",
    ] {
        assert!(message.contains(needle), "{needle:?} in {message}");
    }
}
