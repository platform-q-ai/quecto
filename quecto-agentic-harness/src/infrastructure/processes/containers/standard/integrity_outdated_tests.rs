//! #2206 rounds 2 and 3: a bundle script holding exactly bytes an earlier
//! quecto shipped is `Outdated`. `container doctor` and `container status`
//! refresh it in place; a launch refuses it with the way out, and a
//! retained-argv (teardown) path never runs it — neither ever writes the
//! tree, since an older quecto still running may use those scripts.
//! Unknown bytes are refused as before.

use std::path::{Path, PathBuf};

use super::{EmbeddedScriptIntegrity, RefreshingScriptIntegrity, refuse_altered_script};
use crate::application::environments::dto::AssetState;
use crate::application::environments::ports::ContainerAssetStore;
use crate::application::subagents::dto::StandardScriptVerdict;
use crate::application::subagents::ports::ContainerScriptIntegrity;
use crate::infrastructure::processes::containers::standard::assets::{
    EmbeddedStandardAssets, STANDARD_ASSET_VERSION,
};

/// Every previously shipped script (`bundle path`, the old bytes).
fn shipped() -> [(&'static str, &'static [u8]); 6] {
    [
        (
            "scripts/create.sh",
            include_bytes!(
                "../../../../../tests/fixtures/standard-bundle-v5/create-before-2184.sh"
            ),
        ),
        (
            "scripts/create.sh",
            include_bytes!(
                "../../../../../tests/fixtures/standard-bundle-v5/create-before-2173.sh"
            ),
        ),
        (
            "scripts/inspect.sh",
            include_bytes!("../../../../../tests/fixtures/standard-bundle-v5/inspect.sh"),
        ),
        (
            "scripts/kill.sh",
            include_bytes!("../../../../../tests/fixtures/standard-bundle-v5/kill.sh"),
        ),
        (
            "scripts/create.sh",
            include_bytes!("../../../../../tests/fixtures/standard-bundle-v6/create.sh"),
        ),
        (
            "scripts/exec.sh",
            include_bytes!("../../../../../tests/fixtures/standard-bundle-v6/exec.sh"),
        ),
    ]
}

/// A project with the bundle materialised, then `relative` overwritten
/// with `bytes`.
fn project_with(relative: &str, bytes: &[u8]) -> (tempfile::TempDir, PathBuf) {
    let project = tempfile::tempdir().unwrap();
    let dir = super::test_support::materialise_bundle(project.path());
    let script = dir.join(relative);
    std::fs::write(&script, bytes).unwrap();
    (project, script)
}

fn embedded(relative: &str) -> Vec<u8> {
    EmbeddedStandardAssets
        .catalogue()
        .assets
        .into_iter()
        .find(|asset| asset.path == relative)
        .unwrap()
        .contents
}

#[test]
fn the_bundle_is_version_7() {
    assert_eq!(STANDARD_ASSET_VERSION, 7);
    assert_eq!(EmbeddedStandardAssets.catalogue().version, 7);
}

#[test]
fn every_previously_shipped_script_is_outdated_never_differs() {
    for (relative, bytes) in shipped() {
        assert_ne!(bytes, embedded(relative).as_slice(), "{relative} is old");
        let (_project, script) = project_with(relative, bytes);
        assert_eq!(
            EmbeddedScriptIntegrity.verify(&script),
            StandardScriptVerdict::Outdated,
            "{relative}"
        );
    }
}

#[test]
fn doctor_and_status_refresh_each_outdated_script_in_place() {
    for (relative, bytes) in shipped() {
        let (_project, script) = project_with(relative, bytes);
        assert_eq!(
            RefreshingScriptIntegrity.verify(&script),
            StandardScriptVerdict::Intact,
            "{relative}"
        );
        assert_eq!(
            std::fs::read(&script).unwrap(),
            embedded(relative),
            "{relative}"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&script).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755, "{relative} stays executable");
        }
    }
}

#[test]
fn a_teardown_path_refuses_outdated_bytes_and_writes_nothing() {
    for (relative, bytes) in shipped() {
        let (project, script) = project_with(relative, bytes);
        let argv = vec![script.display().to_string(), "--op".into(), "kill".into()];
        let refusal = refuse_altered_script(&argv).unwrap_err();
        let dir = project.path().display();
        assert!(
            refusal.contains(&format!(
                "was written by an earlier quecto; run `quecto container status --project {dir}` (or `quecto container init --refresh --project {dir}`) to update it — restart any older quecto that is still running first"
            )),
            "{refusal}"
        );
        assert_eq!(
            std::fs::read(&script).unwrap(),
            bytes,
            "{relative}: untouched"
        );
    }
}

#[test]
fn unknown_bytes_are_refused_by_the_launch_and_the_teardown_alike() {
    let (_project, script) = project_with("scripts/kill.sh", b"#!/bin/sh\necho edited\n");
    assert_eq!(
        RefreshingScriptIntegrity.verify(&script),
        StandardScriptVerdict::Differs
    );
    assert!(refuse_altered_script(&[script.display().to_string()]).is_err());
    assert_eq!(
        std::fs::read(&script).unwrap(),
        b"#!/bin/sh\necho edited\n",
        "an edit is never replaced"
    );
    // An old version of one script is not a valid version of another.
    let old_kill = shipped()[3].1;
    let (_project, script) = project_with("scripts/inspect.sh", old_kill);
    assert_eq!(
        RefreshingScriptIntegrity.verify(&script),
        StandardScriptVerdict::Differs
    );
}

#[test]
fn outdated_bytes_are_observed_outdated_and_init_refreshes_them() {
    for (relative, bytes) in shipped() {
        let (project, script) = project_with(relative, bytes);
        let dir = project.path().join(".quecto/containers/standard");
        let asset = EmbeddedStandardAssets
            .catalogue()
            .assets
            .into_iter()
            .find(|asset| asset.path == relative)
            .unwrap();
        assert_eq!(
            EmbeddedStandardAssets.observe(project.path(), &dir, &asset),
            Ok(AssetState::Outdated)
        );
        // A plain init (no --refresh) replaces what quecto itself wrote.
        EmbeddedStandardAssets
            .materialise(project.path(), &dir, &asset)
            .unwrap();
        assert_eq!(std::fs::read(&script).unwrap(), embedded(relative));
    }
}

#[test]
fn a_script_outside_the_bundle_directory_runs_as_it_is() {
    let elsewhere = tempfile::tempdir().unwrap();
    let script = elsewhere.path().join("kill.sh");
    std::fs::write(&script, shipped()[3].1).unwrap();
    assert_eq!(
        RefreshingScriptIntegrity.verify(Path::new(&script)),
        StandardScriptVerdict::NotStandard
    );
    assert!(refuse_altered_script(&[script.display().to_string()]).is_ok());
}

/// `quecto container status` refreshes a script an earlier quecto wrote,
/// says so, reports it `ok`, and names version 7.
#[test]
fn status_refreshes_an_outdated_script_and_reports_version_7() {
    use crate::application::environments::dto::{
        ContainerRuntimeTarget, DiagnosableContainerConfig, PreflightCheck,
    };
    use crate::application::environments::ports::{
        ContainerConfigLookup, ContainerConfigRoster, ContainerConfigRosterReport,
        ContainerRuntimePreflight,
    };
    use crate::application::environments::use_cases::ContainerStatus;
    struct NoRoster;
    impl ContainerConfigRoster for NoRoster {
        fn roster(&self) -> Result<ContainerConfigRosterReport, String> {
            Ok(ContainerConfigRosterReport::default())
        }
        fn revision(&self) -> String {
            String::new()
        }
    }
    struct NoLookup;
    impl ContainerConfigLookup for NoLookup {
        fn lookup(&self, _: &ContainerRuntimeTarget) -> Result<DiagnosableContainerConfig, String> {
            Err("no entry".into())
        }
    }
    impl ContainerRuntimePreflight for NoLookup {
        fn preflight(&self, _: &DiagnosableContainerConfig) -> Result<Vec<PreflightCheck>, String> {
            Err("no entry".into())
        }
    }
    let (project, script) = project_with("scripts/kill.sh", shipped()[3].1);
    let status = ContainerStatus::new(
        std::sync::Arc::new(EmbeddedStandardAssets),
        std::sync::Arc::new(NoRoster),
        std::sync::Arc::new(NoLookup),
        std::sync::Arc::new(NoLookup),
    )
    .execute(project.path());
    assert_eq!(status.version, 7);
    assert!(
        status
            .assets
            .iter()
            .all(|(_, state)| *state == AssetState::Identical),
        "{:?}",
        status.assets
    );
    assert!(
        status
            .diagnostics
            .iter()
            .any(|line| line.starts_with(&format!("refreshed {}", script.display()))),
        "{:?}",
        status.diagnostics
    );
    assert_eq!(std::fs::read(&script).unwrap(), embedded("scripts/kill.sh"));
}

/// #2206 round 3: the launch's judge (composed for `spawn container:`)
/// refuses outdated bytes — naming `container status` and the restart of
/// any older quecto — and never rewrites them.
#[test]
fn the_launch_refuses_outdated_bytes_and_never_rewrites_them() {
    let integrity = crate::composition::container_configs::build_container_script_integrity();
    for (relative, bytes) in shipped() {
        let (_project, script) = project_with(relative, bytes);
        let verdict = integrity.verify(&script);
        assert_eq!(verdict, StandardScriptVerdict::Outdated, "{relative}");
        let refusal = verdict.refusal(&script).unwrap();
        assert!(
            refusal.contains("run `quecto container status --project "),
            "{refusal}"
        );
        assert!(
            refusal.contains("restart any older quecto that is still running first"),
            "{refusal}"
        );
        assert_eq!(
            std::fs::read(&script).unwrap(),
            bytes,
            "{relative}: untouched"
        );
    }
}

/// `container doctor` resolves its target through the refreshing lookup:
/// an outdated script is refreshed and the target resolves; the launch's
/// lookup over the same config refuses it.
#[test]
fn the_doctor_lookup_refreshes_and_the_launch_lookup_refuses() {
    use crate::application::configuration::dto::ConfigSelection;
    use crate::application::environments::dto::ContainerRuntimeTarget;
    let (project, script) = project_with("scripts/kill.sh", shipped()[3].1);
    let scripts = project.path().join(".quecto/containers/standard/scripts");
    let argv = |name: &str| vec![scripts.join(name).display().to_string()];
    let config = project.path().join("config.json");
    std::fs::write(
        &config,
        serde_json::json!({"container_configs": {"standard": {
            "default": true,
            "create": argv("create.sh"),
            "exec": argv("exec.sh"),
            "inspect": argv("inspect.sh"),
            "kill": argv("kill.sh"),
            "cleanup": argv("kill.sh"),
        }}})
        .to_string(),
    )
    .unwrap();
    let base = project.path().join("base");
    std::fs::create_dir_all(&base).unwrap();
    let target = ContainerRuntimeTarget {
        name: Some("standard".into()),
    };
    let launch = crate::composition::environments::build_container_config_lookup(
        &base,
        Some(ConfigSelection::Explicit(config.clone())),
    );
    let refused = launch.lookup(&target).unwrap_err();
    assert!(
        refused.contains("was written by an earlier quecto"),
        "{refused}"
    );
    assert_eq!(std::fs::read(&script).unwrap(), shipped()[3].1, "untouched");
    let doctor = crate::composition::environments::build_refreshing_container_config_lookup(
        &base,
        Some(ConfigSelection::Explicit(config)),
    );
    doctor.lookup(&target).expect("refreshed, then resolved");
    assert_eq!(std::fs::read(&script).unwrap(), embedded("scripts/kill.sh"));
}

/// `quecto container doctor` (as composed) refreshes an outdated script of
/// the entry it diagnoses. The entry's create is not the bundle's here, so
/// the preflight runs no runtime.
#[test]
fn the_composed_doctor_refreshes_an_outdated_script() {
    use crate::application::configuration::dto::ConfigSelection;
    use crate::application::environments::dto::ContainerRuntimeTarget;
    let (project, script) = project_with("scripts/kill.sh", shipped()[3].1);
    let scripts = project.path().join(".quecto/containers/standard/scripts");
    let config = project.path().join("config.json");
    std::fs::write(
        &config,
        serde_json::json!({"container_configs": {"standard": {
            "default": true,
            "create": ["/bin/false"],
            "exec": ["/bin/false"],
            "cleanup": ["/bin/false"],
            "kill": [scripts.join("kill.sh").display().to_string()],
        }}})
        .to_string(),
    )
    .unwrap();
    let base = project.path().join("base");
    std::fs::create_dir_all(&base).unwrap();
    let doctor = crate::composition::environments::build_container_doctor(
        &base,
        &ConfigSelection::Explicit(config),
    );
    let outcome = doctor.execute(&ContainerRuntimeTarget {
        name: Some("standard".into()),
    });
    assert!(
        !format!("{outcome:?}").contains("was written by an earlier quecto"),
        "{outcome:?}"
    );
    assert_eq!(std::fs::read(&script).unwrap(), embedded("scripts/kill.sh"));
}

/// Review P2 of PR #2234: a refresh authorised by an earlier "outdated"
/// judgement re-judges the bytes when it replaces them. A file edited
/// since then is refused and left exactly as the edit made it.
#[test]
fn an_edit_made_after_the_outdated_judgement_is_never_overwritten() {
    for (relative, bytes) in shipped() {
        let (project, script) = project_with(relative, bytes);
        assert_eq!(
            EmbeddedScriptIntegrity.verify(&script),
            StandardScriptVerdict::Outdated
        );
        // The user edits the script before the refresh runs.
        let edited = [bytes, b"\n# my change\n"].concat();
        std::fs::write(&script, &edited).unwrap();
        let dir = project.path().join(".quecto/containers/standard");
        let asset = EmbeddedStandardAssets
            .catalogue()
            .assets
            .into_iter()
            .find(|asset| asset.path == relative)
            .unwrap();
        let refused = EmbeddedStandardAssets
            .refresh_outdated(project.path(), &dir, &asset)
            .unwrap_err();
        assert!(refused.contains("left as it is"), "{refused}");
        assert_eq!(std::fs::read(&script).unwrap(), edited, "{relative}");
        // Through the launch-side judge: refused, the edit stands.
        assert!(matches!(
            RefreshingScriptIntegrity.verify(&script),
            StandardScriptVerdict::Differs
        ));
        assert_eq!(std::fs::read(&script).unwrap(), edited);
    }
}

/// Append an edit to a `race-probe` project's file at `step`.
fn edit_at(step: &'static str) -> fn(&Path, &str) {
    fn edit(destination: &Path, marker: &str) {
        if destination.to_string_lossy().contains("race-probe") {
            let mut bytes = std::fs::read(destination).unwrap();
            bytes.extend_from_slice(format!("\n# edited at {marker}\n").as_bytes());
            std::fs::write(destination, bytes).unwrap();
        }
    }
    match step {
        "observe" => |d, s| {
            if s == "observe" {
                edit(d, s)
            }
        },
        "snapshot" => |d, s| {
            if s == "snapshot" {
                edit(d, s)
            }
        },
        _ => |d, s| {
            if s == "rename" {
                edit(d, s)
            }
        },
    }
}

/// Every window of a refresh — before the destination is judged, between
/// that judgement and the re-read, between the re-read and the rename: a
/// file edited there is never overwritten, whichever judge (the doctor's,
/// or the store's own `refresh_outdated`) asked for the refresh.
#[test]
fn an_edit_in_any_window_of_a_refresh_is_never_overwritten() {
    let _serial = super::super::assets::PLACE_HOOK_SERIAL
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    for step in ["observe", "snapshot", "rename"] {
        for through_the_judge in [false, true] {
            let project = tempfile::Builder::new()
                .prefix("race-probe")
                .tempdir()
                .unwrap();
            let dir = super::test_support::materialise_bundle(project.path());
            let script = dir.join("scripts/kill.sh");
            std::fs::write(&script, shipped()[3].1).unwrap();
            let asset = EmbeddedStandardAssets
                .catalogue()
                .assets
                .into_iter()
                .find(|asset| asset.path == "scripts/kill.sh")
                .unwrap();
            *super::super::assets::PLACE_HOOK.lock().unwrap() = Some(edit_at(step));
            let verdict = if through_the_judge {
                Some(RefreshingScriptIntegrity.verify(&script))
            } else {
                let _ = EmbeddedStandardAssets.refresh_outdated(project.path(), &dir, &asset);
                None
            };
            *super::super::assets::PLACE_HOOK.lock().unwrap() = None;
            let now = std::fs::read(&script).unwrap();
            assert!(
                now.ends_with(format!("# edited at {step}\n").as_bytes()),
                "{step} (judge: {through_the_judge}): the edit stands, verdict {verdict:?}"
            );
        }
    }
}
