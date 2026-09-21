//! The Containerfile belongs to the project (#2073): `init` and `status` say
//! so, and neither counts it as drift nor replaces it.
use super::e2e_tests::{Rig, status_image_present};

const OWN: &str =
    "FROM docker.io/library/python:3.13-slim\nLABEL ai.quecto.required-tools=\"python3 uv\"\n";

#[test]
fn the_projects_containerfile_survives_init_and_refresh_and_status_stays_ready() {
    let rig = Rig::new();
    let ctx = rig.ctx(status_image_present);
    assert_eq!(rig.run(&ctx, &["container", "init"]).exit_code, 0);
    let containerfile = rig.assets_dir().join("Containerfile");
    std::fs::write(&containerfile, OWN).unwrap();

    for args in [
        &["container", "init"][..],
        &["container", "init", "--refresh"][..],
        &["container", "init", "--refresh", "--dry-run"][..],
    ] {
        let run = rig.run(&ctx, args);
        assert_eq!(run.exit_code, 0, "{args:?}: {}", run.stderr);
        assert!(
            run.stdout.contains(&format!(
                "  kept   {} (this project's own; never replaced — review it before building)\n",
                containerfile.display()
            )),
            "{args:?}: {}",
            run.stdout
        );
        assert!(!run.stdout.contains("differs from"), "{}", run.stdout);
        assert_eq!(std::fs::read_to_string(&containerfile).unwrap(), OWN);
    }

    let status = rig.run(&ctx, &["container", "status"]);
    assert_eq!(status.exit_code, 0, "{}", status.stdout);
    let out = &status.stdout;
    assert!(out.contains("assets:  present (5 of 5, version"), "{out}");
    assert!(
        out.contains(&format!(
            "           Containerfile: this project's own ({})\n",
            containerfile.display()
        )),
        "{out}"
    );
    assert!(out.contains("ready: spawn"), "{out}");
}

#[test]
fn a_missing_script_is_not_ready_and_the_containerfile_is_still_yours() {
    let rig = Rig::new();
    let ctx = rig.ctx(status_image_present);
    assert_eq!(rig.run(&ctx, &["container", "init"]).exit_code, 0);
    let containerfile = rig.assets_dir().join("Containerfile");
    std::fs::write(&containerfile, OWN).unwrap();
    std::fs::remove_file(rig.assets_dir().join("scripts/exec.sh")).unwrap();

    let status = rig.run(&ctx, &["container", "status"]);
    assert_eq!(status.exit_code, 1);
    let out = &status.stdout;
    assert!(
        out.contains("assets:  4 of 5 present, 0 differ from the embedded version"),
        "{out}"
    );
    assert!(
        out.contains(&format!(
            "           yours    {}\n",
            containerfile.display()
        )),
        "{out}"
    );
    assert!(out.contains("           missing  "), "{out}");
}

#[test]
fn a_drifted_script_is_still_listed_beside_the_projects_containerfile() {
    let rig = Rig::new();
    let ctx = rig.ctx(status_image_present);
    assert_eq!(rig.run(&ctx, &["container", "init"]).exit_code, 0);
    let containerfile = rig.assets_dir().join("Containerfile");
    std::fs::write(&containerfile, OWN).unwrap();
    std::fs::write(rig.assets_dir().join("scripts/kill.sh"), "#!/bin/sh\n").unwrap();

    let status = rig.run(&ctx, &["container", "status"]);
    assert_eq!(status.exit_code, 1);
    let out = &status.stdout;
    assert!(
        out.contains("assets:  5 of 5 present, 1 differ from the embedded version"),
        "{out}"
    );
    assert!(
        out.contains(&format!(
            "           yours    {}\n",
            containerfile.display()
        )),
        "{out}"
    );
}

#[test]
fn a_flag_the_entry_never_had_is_reported_as_added_not_rewritten() {
    // A sandbox entry (no origin remote) has no --repo; giving one later is
    // an addition, and the line must not read as replacing "(none)".
    let rig = Rig::new();
    let ctx = rig.ctx(status_image_present);
    // <checkout>/.quecto/containers/standard
    let assets_dir = rig.assets_dir();
    let checkout = assets_dir.ancestors().nth(3).unwrap().to_path_buf();
    let removed = std::process::Command::new("git")
        .arg("-C")
        .arg(&checkout)
        .args(["remote", "remove", "origin"])
        .status()
        .unwrap();
    assert!(removed.success());
    let first = rig.run(&ctx, &["container", "init"]);
    assert_eq!(first.exit_code, 0, "{}", first.stderr);
    let again = rig.run(
        &ctx,
        &[
            "container",
            "init",
            "--repo",
            "https://example.test/later.git",
        ],
    );
    assert_eq!(again.exit_code, 0, "{}", again.stderr);
    assert!(
        again
            .stdout
            .contains("  added:   --repo https://example.test/later.git (the entry had none)\n"),
        "{}",
        again.stdout
    );
    assert!(!again.stdout.contains("(none)"), "{}", again.stdout);
    assert!(!again.stdout.contains("rewrote:"), "{}", again.stdout);
}

#[test]
fn init_says_to_make_a_starter_it_wrote_this_projects_before_building() {
    let rig = Rig::new();
    let ctx = rig.ctx(status_image_present);
    let containerfile = rig.assets_dir().join("Containerfile");
    for args in [
        &["container", "init", "--dry-run"][..],
        &["container", "init"][..],
    ] {
        let first = rig.run(&ctx, args);
        assert_eq!(first.exit_code, 0, "{}", first.stderr);
        let hint = format!(
            "  first: {} is a neutral starter — add this project's toolchain",
            containerfile.display()
        );
        let out = &first.stdout;
        assert!(out.contains(&hint), "{args:?}: {out}");
        assert!(
            out.find(&hint).unwrap() < out.find("  1. build the image").unwrap(),
            "{out}"
        );
    }
    // A Containerfile that was already there is not a starter this run wrote.
    let again = rig.run(&ctx, &["container", "init"]);
    assert!(!again.stdout.contains("  first: "), "{}", again.stdout);
}
