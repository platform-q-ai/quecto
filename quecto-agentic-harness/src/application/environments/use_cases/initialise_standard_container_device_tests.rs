//! `quecto container init --device` (the KVM slice): an allowlisted host
//! device is baked into the standard entry's create argv, so nobody
//! hand-edits a trusted overlay. A re-init without the flag keeps the
//! entry's devices; the flag (or `--no-device`, an empty list) replaces
//! them; anything off the allowlist is refused before anything is written.
use super::rig_tests::*;
use crate::application::environments::dto::{
    ALLOWED_CONTAINER_DEVICES, EntryValueChange, InitialiseStandardContainerError,
};
use crate::application::environments::ports::container::ContainerConfigRosterReport;

fn with_devices(
    devices: &[&str],
) -> crate::application::environments::dto::StandardContainerRequest {
    let mut request = request("/p");
    request.devices = Some(devices.iter().map(|d| (*d).to_string()).collect());
    request
}

fn rig() -> Rig {
    build_rig(Ok(None), ContainerConfigRosterReport::default(), None)
}

#[test]
fn the_allowlist_is_kvm_alone() {
    assert_eq!(ALLOWED_CONTAINER_DEVICES, ["/dev/kvm"]);
}

#[test]
fn an_allowlisted_device_is_baked_into_the_create_argv_after_the_image() {
    let rig = rig();
    let report = rig
        .use_case
        .execute(&with_devices(&["/dev/kvm", "/dev/kvm"]))
        .unwrap();
    assert_eq!(report.devices, ["/dev/kvm"], "a repeat is one device");
    assert_eq!(report.devices_change, None, "no entry to keep from");
    let create = argv(&report.entry.entry, "create");
    assert_eq!(create[create.len() - 2..], ["--device", "/dev/kvm"]);
    assert_eq!(create[3..5], ["--image".to_string(), report.image.clone()]);
    let written = rig.persistence.written.lock().unwrap();
    assert_eq!(written.last().unwrap().1, report.entry.entry);
    // Only create carries it: the other argvs never take a device.
    for key in ["exec", "inspect", "kill", "cleanup"] {
        assert!(
            !argv(&report.entry.entry, key).contains(&"--device".to_string()),
            "{key}"
        );
    }
}

#[test]
fn no_device_flag_on_a_first_init_writes_none() {
    let report = rig().use_case.execute(&request("/p")).unwrap();
    assert!(report.devices.is_empty());
    assert!(!argv(&report.entry.entry, "create").contains(&"--device".to_string()));
}

#[test]
fn a_device_off_the_allowlist_is_refused_before_anything_is_written() {
    for device in ["/dev/sda", "dev/kvm", "/dev/kvm/", "/dev/../dev/kvm", ""] {
        let rig = rig();
        let error = rig
            .use_case
            .execute(&with_devices(&["/dev/kvm", device]))
            .unwrap_err();
        assert_eq!(
            error,
            InitialiseStandardContainerError::DeviceNotAllowed(device.to_string())
        );
        assert_eq!(error.to_string(), "--device: only /dev/kvm is supported");
        assert!(
            rig.persistence.written.lock().unwrap().is_empty(),
            "{device:?}"
        );
        assert!(rig.assets.disk.lock().unwrap().is_empty(), "{device:?}");
    }
}

#[test]
fn a_re_init_keeps_the_entrys_devices_unless_the_flag_replaces_them() {
    let rig = rig();
    rig.use_case.execute(&with_devices(&["/dev/kvm"])).unwrap();

    // No flag (a `--refresh` too): kept, and said so.
    let mut refresh = request("/p");
    refresh.refresh = true;
    let kept = rig.use_case.execute(&refresh).unwrap();
    assert_eq!(kept.devices, ["/dev/kvm"]);
    assert_eq!(kept.devices_change, Some(EntryValueChange::Kept));
    assert!(argv(&kept.entry.entry, "create").ends_with(&["--device".into(), "/dev/kvm".into()]));

    // The same list again is a keep.
    let again = rig.use_case.execute(&with_devices(&["/dev/kvm"])).unwrap();
    assert_eq!(again.devices_change, Some(EntryValueChange::Kept));

    // An empty list (`--no-device`) replaces it with none.
    let cleared = rig.use_case.execute(&with_devices(&[])).unwrap();
    assert!(cleared.devices.is_empty());
    assert_eq!(
        cleared.devices_change,
        Some(EntryValueChange::Rewrote {
            previous: Some("/dev/kvm".into())
        })
    );
    assert!(!argv(&cleared.entry.entry, "create").contains(&"--device".to_string()));

    // And the flag adds it back to an entry that had none.
    let added = rig.use_case.execute(&with_devices(&["/dev/kvm"])).unwrap();
    assert_eq!(
        added.devices_change,
        Some(EntryValueChange::Rewrote { previous: None })
    );
}

#[test]
fn a_kept_device_is_judged_against_the_allowlist_too() {
    let rig = rig();
    rig.persistence.written.lock().unwrap().push((
        "standard".into(),
        crate::application::environments::dto::ContainerConfigDocument {
            create: vec![
                "/p/.quecto/containers/standard/scripts/create.sh".into(),
                "--device".into(),
                "/dev/sda".into(),
            ],
            ..Default::default()
        },
    ));
    let error = rig.use_case.execute(&request("/p")).unwrap_err();
    assert_eq!(
        error,
        InitialiseStandardContainerError::DeviceNotAllowed("/dev/sda".into())
    );
    // Replacing the list is the way out.
    assert!(rig.use_case.execute(&with_devices(&[])).is_ok());
}
