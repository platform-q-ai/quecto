//! `quecto container status` presenters over the real composition (#2024
//! S4e), continued from `container_setup_e2e_tests.rs` (whose rig this
//! shares): the entry's layer, a withheld overlay, per-asset lines, a
//! warned image.

use super::*;

#[test]
fn a_global_standard_entry_is_reported_as_global_with_no_trust_to_speak_of() {
    let rig = Rig::new();
    std::fs::write(
        rig.base_dir.join("config.json"),
        serde_json::json!({"container_configs": {"standard": {"default": true, "create": ["/opt/std/create.sh", "--repo", "https://example.test/g.git"]}}}).to_string(),
    )
    .unwrap();
    let ctx = rig.ctx(status_image_present);
    let status = rig.run(&ctx, &["container", "status"]);
    let out = &status.stdout;
    assert!(out.contains("config:  standard (default, global) in the effective configuration; --repo https://example.test/g.git"), "{out}");
    assert!(
        out.contains("trust:   n/a (the entry is declared globally)"),
        "{out}"
    );
    assert_eq!(status.exit_code, 1, "assets are still missing: {out}");
}

#[test]
fn an_untrusted_overlay_withholds_the_config_and_refuses_init() {
    let rig = Rig::new();
    std::fs::create_dir_all(rig.checkout.join(".quecto")).unwrap();
    std::fs::write(
        rig.checkout.join(".quecto/config.json"),
        r#"{"container_configs":{"standard":{"create":["/x"]}}}"#,
    )
    .unwrap();
    let ctx = rig.ctx(status_image_present);
    let status = rig.run(&ctx, &["container", "status"]);
    assert_eq!(status.exit_code, 1);
    let out = &status.stdout;
    assert!(
        out.contains("config:  unknown — the repo-local overlay was not applied (see trust)"),
        "{out}"
    );
    assert!(
        out.contains("trust:   withheld — the repo-local overlay is not trusted"),
        "{out}"
    );
    assert!(
        out.contains("note:    "),
        "the layer diagnostic is a note: {out}"
    );
    let init = rig.run(&ctx, &["container", "init"]);
    assert_eq!(init.exit_code, 1);
    assert!(init.stderr.contains("not trusted"), "{}", init.stderr);
    assert!(!rig.assets_dir().exists());
}

#[cfg(unix)]
#[test]
fn status_lists_a_missing_and_a_refused_asset_by_name() {
    let rig = Rig::new();
    let ctx = rig.ctx(status_image_warned);
    assert_eq!(rig.run(&ctx, &["container", "init"]).exit_code, 0);
    let assets_dir = rig.assets_dir();
    std::fs::remove_file(assets_dir.join("scripts/exec.sh")).unwrap();
    std::fs::remove_file(assets_dir.join("Containerfile")).unwrap();
    std::os::unix::fs::symlink(
        assets_dir.join("scripts/kill.sh"),
        assets_dir.join("Containerfile"),
    )
    .unwrap();
    let project = rig.checkout.to_string_lossy().into_owned();
    let status = rig.run(&ctx, &["container", "status", "--project", &project]);
    assert_eq!(status.exit_code, 1);
    let out = &status.stdout;
    assert!(
        out.contains("assets:  3 of 5 present, 0 differ from the embedded version"),
        "{out}"
    );
    assert!(
        out.contains(&format!(
            "           missing  {}\n",
            assets_dir.join("scripts/exec.sh").display()
        )),
        "{out}"
    );
    assert!(
        out.contains(&format!(
            "           refused  {}\n",
            assets_dir.join("Containerfile").display()
        )),
        "{out}"
    );
    assert!(
        out.contains("note:    ") && out.contains("symbolic link"),
        "{out}"
    );
    // The lookup refuses the bundle with a script missing, before any
    // preflight is asked.
    assert!(
        out.contains("image:   unknown — container config 'standard' refused:")
            && out.contains("exec.sh is missing from the standard bundle"),
        "{out}"
    );
    assert!(out.contains("not ready"), "{out}");

    // A dry refresh says what it would replace, and replaces nothing.
    std::fs::write(assets_dir.join("scripts/inspect.sh"), "edited").unwrap();
    let dry = rig.run(&ctx, &["container", "init", "--dry-run", "--refresh"]);
    assert_eq!(
        dry.exit_code, 1,
        "the symbolic link refuses the run: {}",
        dry.stdout
    );
    assert!(dry.stderr.contains("symbolic link"), "{}", dry.stderr);
    std::fs::remove_file(assets_dir.join("Containerfile")).unwrap();
    let dry = rig.run(&ctx, &["container", "init", "--dry-run", "--refresh"]);
    assert_eq!(dry.exit_code, 0, "{}", dry.stderr);
    assert!(
        dry.stdout.contains(&format!(
            "  would refresh  {} (replaced with the embedded version",
            assets_dir.join("scripts/inspect.sh").display()
        )),
        "{}",
        dry.stdout
    );
    assert_eq!(
        std::fs::read_to_string(assets_dir.join("scripts/inspect.sh")).unwrap(),
        "edited"
    );
    assert!(!assets_dir.join("Containerfile").exists());
}

#[test]
fn a_warned_image_check_is_shown_with_its_remedy_and_is_not_ready() {
    let rig = Rig::new();
    let ctx = rig.ctx(status_image_warned);
    assert_eq!(rig.run(&ctx, &["container", "init"]).exit_code, 0);
    let status = rig.run(&ctx, &["container", "status"]);
    assert_eq!(status.exit_code, 1);
    let out = &status.stdout;
    assert!(
        out.contains("image:   ! image quecto-box:local is stale\n           remedy: rebuild it\n"),
        "{out}"
    );
    assert!(out.contains("not ready"), "{out}");
}
