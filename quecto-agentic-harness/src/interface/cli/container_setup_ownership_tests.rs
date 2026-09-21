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
                "  kept   {} (this project's own; never replaced)\n",
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
    assert!(out.contains("ready"), "{out}");
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
