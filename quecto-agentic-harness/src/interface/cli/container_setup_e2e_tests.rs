//! `quecto container init|status` through the real composition (#2024
//! S4e): the CLI over a hermetic checkout with the embedded bundle, the
//! checkout's git origin, the configuration capability's overlay write
//! and trust — the same graph `main` wires — so the presenters are judged
//! on the report a real init produces. Status is composed over a stub
//! preflight (the entry's create script would otherwise probe the
//! runtime), which is the one port the interface never names.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::*;
use crate::application::configuration::dto::ConfigSelection;
use crate::application::environments::dto::{
    DiagnosableContainerConfig, PreflightCheck, STANDARD_CONTAINER_DIR,
};
use crate::application::environments::ports::{ContainerAssetStore, ContainerRuntimePreflight};
use crate::application::environments::use_cases::ContainerStatus;
use crate::composition::standard_container::build_standard_container_init;
use crate::infrastructure::processes::containers::standard::assets::EmbeddedStandardAssets;
use crate::interface::cli::{CliOutput, ContainerStatusBuilder};

pub(super) struct Rig {
    pub(super) _dir: tempfile::TempDir,
    pub(super) base_dir: PathBuf,
    pub(super) checkout: PathBuf,
}

pub(super) fn git(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

impl Rig {
    pub(super) fn new() -> Self {
        let dir = tempfile::TempDir::new().unwrap();
        let base_dir = dir.path().join("base").canonicalize_after_create();
        let checkout = dir.path().join("checkout").canonicalize_after_create();
        std::fs::write(base_dir.join("config.json"), "{}").unwrap();
        git(&checkout, &["init", "-q"]);
        git(
            &checkout,
            &[
                "remote",
                "add",
                "origin",
                "https://example.test/org/repo.git",
            ],
        );
        Self {
            _dir: dir,
            base_dir,
            checkout,
        }
    }

    pub(super) fn ctx(&self, status: ContainerStatusBuilder) -> CliContext {
        CliContext {
            base_dir: Some(self.base_dir.clone()),
            cwd: Some(self.checkout.clone()),
            configuration: Some(crate::composition::configuration::build_configuration_handles),
            container_init: Some(build_standard_container_init),
            container_status: Some(status),
            ..Default::default()
        }
    }

    pub(super) fn run(&self, ctx: &CliContext, args: &[&str]) -> CliOutput {
        let mut argv = vec!["quecto".to_string()];
        argv.extend(args.iter().map(|s| s.to_string()));
        crate::interface::cli::run_with_output(argv, ctx)
    }

    pub(super) fn assets_dir(&self) -> PathBuf {
        self.checkout.join(STANDARD_CONTAINER_DIR)
    }

    pub(super) fn overlay(&self) -> serde_json::Value {
        serde_json::from_str(
            &std::fs::read_to_string(self.checkout.join(".quecto/config.json")).unwrap(),
        )
        .unwrap()
    }
}

trait CreateCanonical {
    fn canonicalize_after_create(self) -> PathBuf;
}

impl CreateCanonical for PathBuf {
    fn canonicalize_after_create(self) -> PathBuf {
        std::fs::create_dir_all(&self).unwrap();
        self.canonicalize().unwrap()
    }
}

struct ImagePresent;

impl ContainerRuntimePreflight for ImagePresent {
    fn preflight(
        &self,
        config: &DiagnosableContainerConfig,
    ) -> Result<Vec<PreflightCheck>, String> {
        assert_eq!(config.name, "standard");
        assert!(
            config.create[0].ends_with("scripts/create.sh"),
            "{:?}",
            config.create
        );
        Ok(vec![
            PreflightCheck {
                name: "runtime".into(),
                status: CheckStatus::Passed,
                detail: "podman".into(),
                remedy: String::new(),
            },
            PreflightCheck {
                name: "image".into(),
                status: CheckStatus::Passed,
                detail: "image quecto-box:local present".into(),
                remedy: String::new(),
            },
        ])
    }
}

struct ImageMissing;

impl ContainerRuntimePreflight for ImageMissing {
    fn preflight(&self, _: &DiagnosableContainerConfig) -> Result<Vec<PreflightCheck>, String> {
        Ok(vec![PreflightCheck {
            name: "image".into(),
            status: CheckStatus::Failed,
            detail: "image quecto-box:local not found".into(),
            remedy: "build it".into(),
        }])
    }
}

struct PreflightRefused;

impl ContainerRuntimePreflight for PreflightRefused {
    fn preflight(&self, _: &DiagnosableContainerConfig) -> Result<Vec<PreflightCheck>, String> {
        Err("the create script refuses --preflight-only".into())
    }
}

fn status_over(
    base_dir: &Path,
    selection: &ConfigSelection,
    preflight: Arc<dyn ContainerRuntimePreflight>,
) -> Arc<ContainerStatus> {
    Arc::new(ContainerStatus::new(
        Arc::new(EmbeddedStandardAssets),
        crate::composition::container_configs::build_container_config_roster(
            base_dir,
            Some(selection.clone()),
        ),
        crate::composition::environments::build_container_config_lookup(
            base_dir,
            Some(selection.clone()),
        ),
        preflight,
    ))
}

pub(super) fn status_image_present(
    base_dir: &Path,
    selection: &ConfigSelection,
) -> Arc<ContainerStatus> {
    status_over(base_dir, selection, Arc::new(ImagePresent))
}

fn status_image_missing(base_dir: &Path, selection: &ConfigSelection) -> Arc<ContainerStatus> {
    status_over(base_dir, selection, Arc::new(ImageMissing))
}

fn status_preflight_refused(base_dir: &Path, selection: &ConfigSelection) -> Arc<ContainerStatus> {
    status_over(base_dir, selection, Arc::new(PreflightRefused))
}

#[test]
fn status_before_init_reports_everything_missing_and_exits_1() {
    let rig = Rig::new();
    let ctx = rig.ctx(status_image_present);
    let output = rig.run(&ctx, &["container", "status"]);
    assert_eq!(output.exit_code, 1, "{}", output.stderr);
    let out = &output.stdout;
    assert!(
        out.contains(&format!(
            "standard container at {}",
            rig.assets_dir().display()
        )),
        "{out}"
    );
    assert!(
        out.contains("assets:  missing — run `quecto container init`"),
        "{out}"
    );
    assert!(out.contains("config:  standard entry missing"), "{out}");
    assert!(out.contains("trust:   n/a (no overlay entry)"), "{out}");
    assert!(
        out.contains("image:   not checked (no entry to ask)"),
        "{out}"
    );
    assert!(
        out.ends_with("not ready: fix the lines above, then `quecto container doctor`\n"),
        "{out}"
    );
}

#[test]
fn init_materialises_the_bundle_writes_the_trusted_entry_and_status_is_then_ready() {
    let rig = Rig::new();
    let ctx = rig.ctx(status_image_present);

    let dry = rig.run(&ctx, &["container", "init", "--dry-run"]);
    assert_eq!(dry.exit_code, 0, "{}", dry.stderr);
    assert!(dry.stdout.contains("would write"), "{}", dry.stdout);
    assert!(
        dry.stdout
            .contains("would be written as container_configs.standard"),
        "{}",
        dry.stdout
    );
    assert!(
        !dry.stdout.contains("trusted for exactly these bytes"),
        "{}",
        dry.stdout
    );
    assert!(!rig.assets_dir().exists(), "a dry run writes nothing");
    assert!(!rig.checkout.join(".quecto/config.json").exists());

    let output = rig.run(&ctx, &["container", "init"]);
    assert_eq!(output.exit_code, 0, "{}", output.stderr);
    let out = &output.stdout;
    let assets_dir = rig.assets_dir();
    assert!(
        out.contains(&format!(
            "bundle (version {}) at {}",
            EmbeddedStandardAssets.catalogue().version,
            assets_dir.display()
        )),
        "{out}"
    );
    for name in [
        "Containerfile",
        "scripts/create.sh",
        "scripts/exec.sh",
        "scripts/inspect.sh",
        "scripts/kill.sh",
    ] {
        assert!(
            out.contains(&format!("  wrote  {}\n", assets_dir.join(name).display())),
            "{name}: {out}"
        );
        assert!(assets_dir.join(name).is_file(), "{name}");
    }
    assert!(
        out.contains("written as container_configs.standard in"),
        "{out}"
    );
    assert!(out.contains("(trusted for exactly these bytes)"), "{out}");
    assert!(
        out.contains("default: true — `spawn container: true` selects it"),
        "{out}"
    );
    assert!(
        out.contains("--repo https://example.test/org/repo.git (the checkout's origin remote)"),
        "{out}"
    );
    assert!(out.contains("--image quecto-box:local"), "{out}");
    assert!(
        !out.contains("kept:"),
        "no existing entry to keep from: {out}"
    );
    assert!(
        out.contains(&format!(
            "podman build -t quecto-box:local -f {0}/Containerfile {0}",
            assets_dir.display()
        )),
        "{out}"
    );
    assert!(
        out.contains("2. quecto container doctor   — every check ✓"),
        "{out}"
    );
    let entry = &rig.overlay()["container_configs"]["standard"];
    assert_eq!(entry["default"], true);
    assert_eq!(
        entry["create"][0],
        assets_dir
            .join("scripts/create.sh")
            .to_string_lossy()
            .as_ref()
    );

    let status = rig.run(&ctx, &["container", "status"]);
    assert_eq!(status.exit_code, 0, "{}\n{}", status.stdout, status.stderr);
    let out = &status.stdout;
    assert!(out.contains("assets:  present (5 of 5, version"), "{out}");
    assert!(out.contains("config:  standard (default, overlay) in the effective configuration; --repo https://example.test/org/repo.git"), "{out}");
    assert!(
        out.contains("trust:   trusted (the repo-local overlay is applied)"),
        "{out}"
    );
    assert!(
        out.contains("image:   image quecto-box:local present\n"),
        "{out}"
    );
    assert!(!out.contains("remedy:"), "{out}");
    assert!(
        out.ends_with("ready: spawn {\"container\":true} from an agent in this project\n"),
        "{out}"
    );
}

#[test]
fn a_re_init_keeps_the_entrys_values_and_reports_an_edited_asset_until_refreshed() {
    let rig = Rig::new();
    let ctx = rig.ctx(status_image_missing);
    assert_eq!(
        rig.run(
            &ctx,
            &[
                "container",
                "init",
                "--repo",
                "https://example.test/other.git",
                "--image",
                "box:v1"
            ]
        )
        .exit_code,
        0
    );
    let create = rig.assets_dir().join("scripts/create.sh");
    std::fs::write(&create, "#!/bin/sh\nexit 0\n").unwrap();

    let again = rig.run(&ctx, &["container", "init"]);
    assert_eq!(again.exit_code, 0, "{}", again.stderr);
    let out = &again.stdout;
    assert!(
        out.contains(&format!(
            "  kept   {} (differs from the embedded version",
            create.display()
        )),
        "{out}"
    );
    assert!(
        out.contains("no files changed (existing files are never replaced without --refresh)"),
        "{out}"
    );
    assert!(
        out.contains("--repo https://example.test/other.git (the existing entry's)"),
        "{out}"
    );
    assert!(out.contains("kept:    --repo https://example.test/other.git (the existing entry's; pass --repo to change it)"), "{out}");
    assert!(
        out.contains("kept:    --image box:v1 (the existing entry's; pass --image to change it)"),
        "{out}"
    );
    assert_eq!(
        std::fs::read_to_string(&create).unwrap(),
        "#!/bin/sh\nexit 0\n"
    );

    let status = rig.run(&ctx, &["container", "status"]);
    assert_eq!(status.exit_code, 1);
    let out = &status.stdout;
    assert!(
        out.contains("assets:  5 of 5 present, 1 differ from the embedded version"),
        "{out}"
    );
    assert!(
        out.contains(&format!("           differs  {}\n", create.display())),
        "{out}"
    );
    assert!(
        out.contains(&format!(
            "           ok       {}\n",
            rig.assets_dir().join("Containerfile").display()
        )),
        "{out}"
    );
    // The lookup itself refuses an edited standard script, before any
    // preflight is asked: the image line says so.
    assert!(
        out.contains("image:   unknown — container config 'standard' refused:"),
        "{out}"
    );
    assert!(out.contains("differs from the standard bundle"), "{out}");
    assert!(out.contains("not ready"), "{out}");

    let refreshed = rig.run(
        &ctx,
        &["container", "init", "--refresh", "--image", "box:v2"],
    );
    assert_eq!(refreshed.exit_code, 0, "{}", refreshed.stderr);
    let out = &refreshed.stdout;
    assert!(
        out.contains(&format!(
            "  refreshed  {} (replaced with the embedded version",
            create.display()
        )),
        "{out}"
    );
    assert!(
        out.contains("rewrote: --image box:v2 (was box:v1)"),
        "{out}"
    );
    assert!(!out.contains("no files changed"), "{out}");
    assert_eq!(
        std::fs::read(&create).unwrap(),
        EmbeddedStandardAssets.catalogue().assets[1].contents
    );

    let status = rig.run(&ctx, &["container", "status"]);
    assert_eq!(status.exit_code, 1);
    let out = &status.stdout;
    assert!(out.contains("assets:  present (5 of 5, version"), "{out}");
    assert!(
        out.contains("image:   ✗ image quecto-box:local not found\n           remedy: build it\n"),
        "{out}"
    );

    let dry = rig.run(
        &ctx,
        &[
            "container",
            "init",
            "--dry-run",
            "--repo",
            "https://example.test/third.git",
        ],
    );
    assert_eq!(dry.exit_code, 0, "{}", dry.stderr);
    assert!(
        dry.stdout
            .contains("--repo https://example.test/third.git (as given)"),
        "{}",
        dry.stdout
    );
    assert!(
        dry.stdout.contains(
            "rewrote: --repo https://example.test/third.git (was https://example.test/other.git)"
        ),
        "{}",
        dry.stdout
    );
    assert!(dry.stdout.contains("  kept   "), "{}", dry.stdout);
    assert_eq!(
        rig.overlay()["container_configs"]["standard"]["create"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v == &"https://example.test/third.git")
            .count(),
        0,
        "a dry run rewrites nothing"
    );
}

#[test]
fn a_sandbox_checkout_and_a_global_default_are_both_said_and_the_preflight_refusal_is_a_note() {
    let rig = Rig::new();
    git(&rig.checkout, &["remote", "remove", "origin"]);
    std::fs::write(
        rig.base_dir.join("config.json"),
        serde_json::json!({"container_configs": {"corp": {"default": true, "create": ["/opt/corp/create.sh"]}}}).to_string(),
    )
    .unwrap();
    let ctx = rig.ctx(status_preflight_refused);
    let output = rig.run(&ctx, &["container", "init"]);
    assert_eq!(output.exit_code, 0, "{}", output.stderr);
    let out = &output.stdout;
    assert!(
        out.contains("sandbox: no --repo (the checkout has no origin remote and none was given)"),
        "{out}"
    );
    assert!(out.contains("not the default: corp is already the default; select it with container: {\"mode\":\"new\",\"container_config\":\"standard\"}"), "{out}");
    assert!(
        out.contains("2. quecto container doctor --name standard   — every check ✓"),
        "{out}"
    );
    assert!(
        out.contains(
            "3. spawn {\"agent_id\":\"probe\",\"task\":\"run pwd\",\"container\":{\"mode\":\"new\",\"container_config\":\"standard\"}}"
        ),
        "{out}"
    );
    assert!(
        rig.overlay()["container_configs"]["standard"]
            .get("default")
            .is_none(),
        "{}",
        rig.overlay()
    );

    let again = rig.run(&ctx, &["container", "init"]);
    assert!(
        again
            .stdout
            .contains("sandbox: no --repo (the existing entry has none)"),
        "{}",
        again.stdout
    );
    assert!(
        again
            .stdout
            .contains("kept:    --repo (none) (the existing entry's"),
        "{}",
        again.stdout
    );

    let status = rig.run(&ctx, &["container", "status"]);
    assert_eq!(status.exit_code, 1);
    let out = &status.stdout;
    assert!(out.contains("config:  standard (not default, overlay) in the effective configuration; sandbox (no --repo)"), "{out}");
    assert!(
        out.contains("image:   unknown — the create script refuses --preflight-only"),
        "{out}"
    );
    assert!(
        !out.contains("note:    the create script refuses"),
        "the preflight error is not repeated as a note: {out}"
    );
}

#[test]
fn project_selection_refuses_config_and_a_project_that_is_not_the_root_or_not_there() {
    let rig = Rig::new();
    let mut ctx = rig.ctx(status_image_present);
    ctx.config_path = Some(rig.base_dir.join("config.json"));
    for command in ["init", "status"] {
        let output = rig.run(&ctx, &["container", command]);
        assert_eq!(output.exit_code, 1);
        assert!(
            output.stderr.contains("run without --config"),
            "{}",
            output.stderr
        );
    }
    let ctx = rig.ctx(status_image_present);
    let nowhere = rig.checkout.join("nowhere").to_string_lossy().into_owned();
    let output = rig.run(&ctx, &["container", "init", "--project", &nowhere]);
    assert_eq!(output.exit_code, 1);
    assert!(
        output.stderr.contains("is not accessible"),
        "{}",
        output.stderr
    );
    let sub = rig.checkout.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let output = rig.run(
        &ctx,
        &["container", "init", "--project", &sub.to_string_lossy()],
    );
    assert_eq!(output.exit_code, 1);
    assert!(
        output
            .stderr
            .contains(&format!("pass --project {}", rig.checkout.display())),
        "{}",
        output.stderr
    );
    let mut no_cwd = rig.ctx(status_image_present);
    no_cwd.cwd = None;
    let output = rig.run(&no_cwd, &["container", "status"]);
    assert_eq!(output.exit_code, 1);
    assert!(
        output
            .stderr
            .contains("the working directory is unknown; pass --project"),
        "{}",
        output.stderr
    );
    // `--project` elsewhere than the working directory selects that
    // project's overlay.
    let mut elsewhere = rig.ctx(status_image_present);
    elsewhere.cwd = Some(rig.base_dir.clone());
    let output = rig.run(
        &elsewhere,
        &[
            "container",
            "init",
            "--project",
            &rig.checkout.to_string_lossy(),
        ],
    );
    assert_eq!(output.exit_code, 0, "{}", output.stderr);
    assert!(rig.checkout.join(".quecto/config.json").is_file());
    assert!(!rig.base_dir.join(".quecto").exists());
}

#[test]
fn the_base_dir_is_made_absolute_canonical_when_it_exists_and_as_given_when_absolute() {
    let rig = Rig::new();
    let ctx = rig.ctx(status_image_present);
    assert_eq!(absolute_base_dir(&ctx).unwrap(), rig.base_dir);
    let mut absent = ctx.clone();
    absent.base_dir = Some(rig.base_dir.join("not-yet"));
    assert_eq!(
        absolute_base_dir(&absent).unwrap(),
        rig.base_dir.join("not-yet")
    );
    let mut relative = ctx.clone();
    relative.base_dir = Some(PathBuf::from("relative-base-that-does-not-exist"));
    let resolved = absolute_base_dir(&relative).unwrap();
    assert!(resolved.is_absolute(), "{}", resolved.display());
    assert!(resolved.ends_with("relative-base-that-does-not-exist"));
}

struct ImageWarned;

impl ContainerRuntimePreflight for ImageWarned {
    fn preflight(&self, _: &DiagnosableContainerConfig) -> Result<Vec<PreflightCheck>, String> {
        Ok(vec![PreflightCheck {
            name: "image".into(),
            status: CheckStatus::Warned,
            detail: "image quecto-box:local is stale".into(),
            remedy: "rebuild it".into(),
        }])
    }
}

pub(super) fn status_image_warned(
    base_dir: &Path,
    selection: &ConfigSelection,
) -> Arc<ContainerStatus> {
    status_over(base_dir, selection, Arc::new(ImageWarned))
}

#[cfg(test)]
#[path = "container_setup_status_tests.rs"]
mod status_tests;
