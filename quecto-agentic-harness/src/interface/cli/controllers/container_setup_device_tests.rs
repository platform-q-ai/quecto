//! `quecto container init --device /dev/kvm` through the real composition:
//! the trusted overlay it writes carries the device in the standard create
//! argv, a re-init keeps it, `--no-device` removes it, and a device off the
//! allowlist is refused with the overlay untouched.

use super::e2e_tests::{Rig, status_image_present};

fn create_argv(rig: &Rig) -> Vec<String> {
    rig.overlay()["container_configs"]["standard"]["create"]
        .as_array()
        .expect("the standard entry's create argv")
        .iter()
        .map(|arg| arg.as_str().unwrap().to_string())
        .collect()
}

fn device_pairs(argv: &[String]) -> Vec<&str> {
    argv.windows(2)
        .filter(|pair| pair[0] == "--device")
        .map(|pair| pair[1].as_str())
        .collect()
}

#[test]
fn init_writes_the_device_into_the_trusted_overlay_and_a_re_init_keeps_it() {
    let rig = Rig::new();
    let ctx = rig.ctx(status_image_present);

    let output = rig.run(&ctx, &["container", "init", "--device", "/dev/kvm"]);
    assert_eq!(output.exit_code, 0, "{}", output.stderr);
    assert!(
        output.stdout.contains(
            "  --device /dev/kvm: the container gets this host device (`quecto container doctor` checks this user can read and write it)\n"
        ),
        "{}",
        output.stdout
    );
    assert_eq!(device_pairs(&create_argv(&rig)), ["/dev/kvm"]);
    // The overlay is trusted for exactly these bytes: status applies it.
    let status = rig.run(&ctx, &["container", "status"]);
    assert_eq!(status.exit_code, 0, "{}\n{}", status.stdout, status.stderr);
    assert!(
        status.stdout.contains("trust:   trusted"),
        "{}",
        status.stdout
    );

    let refresh = rig.run(&ctx, &["container", "init", "--refresh"]);
    assert_eq!(refresh.exit_code, 0, "{}", refresh.stderr);
    assert!(
        refresh.stdout.contains(
            "  kept:    --device /dev/kvm (the existing entry's; pass --device to change it)\n"
        ),
        "{}",
        refresh.stdout
    );
    assert_eq!(device_pairs(&create_argv(&rig)), ["/dev/kvm"]);

    let removed = rig.run(&ctx, &["container", "init", "--no-device", "--refresh"]);
    assert_eq!(removed.exit_code, 0, "{}", removed.stderr);
    assert!(
        removed
            .stdout
            .contains("  rewrote: --device (none) (was /dev/kvm)\n"),
        "{}",
        removed.stdout
    );
    assert!(device_pairs(&create_argv(&rig)).is_empty());

    let added = rig.run(&ctx, &["container", "init", "--device", "/dev/kvm"]);
    assert_eq!(added.exit_code, 0, "{}", added.stderr);
    assert!(
        added
            .stdout
            .contains("  added:   --device /dev/kvm (the entry had none)\n"),
        "{}",
        added.stdout
    );
}

#[test]
fn a_device_off_the_allowlist_is_refused_and_the_overlay_is_untouched() {
    let rig = Rig::new();
    let ctx = rig.ctx(status_image_present);
    for device in ["/dev/sda", "dev/kvm"] {
        let output = rig.run(&ctx, &["container", "init", "--device", device]);
        assert_eq!(output.exit_code, 1, "{device}");
        assert_eq!(
            output.stderr,
            format!(
                "--device: only /dev/kvm is supported, not {device:?} (given with --device); pass --device /dev/kvm, or --no-device for none\n"
            )
        );
        assert!(!rig.checkout.join(".quecto/config.json").exists());
        assert!(!rig.assets_dir().exists());
    }
}

#[test]
fn the_device_flags_parse_strictly() {
    let rig = Rig::new();
    let ctx = rig.ctx(status_image_present);
    for (args, needle) in [
        (
            &["container", "init", "--device"][..],
            "--device requires a value",
        ),
        (
            &["container", "init", "--device", ""][..],
            "--device requires a value",
        ),
        (
            &["container", "init", "--device", "/dev/kvm", "--no-device"][..],
            "--device and --no-device cannot be combined",
        ),
        (
            &["container", "init", "--no-device", "--device", "/dev/kvm"][..],
            "--device and --no-device cannot be combined",
        ),
    ] {
        let output = rig.run(&ctx, args);
        assert_eq!(output.exit_code, 1, "{args:?}");
        assert!(
            output.stderr.contains(needle),
            "{args:?}: {}",
            output.stderr
        );
        assert!(
            output
                .stderr
                .contains("[--device /dev/kvm]... [--no-device]"),
            "the usage names the flags: {}",
            output.stderr
        );
    }
}
