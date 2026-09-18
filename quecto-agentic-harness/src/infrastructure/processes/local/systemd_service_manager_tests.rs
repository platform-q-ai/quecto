use super::*;
use std::os::unix::fs::PermissionsExt;

/// A fake `systemctl` that appends its argv to a log file (`$FAKE_LOG`) and
/// exits successfully, so the manager's calls are observable without touching
/// the real user session.
fn fake_systemctl(dir: &std::path::Path, log: &std::path::Path) -> PathBuf {
    let script = dir.join("systemctl");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

fn manager(tmp: &tempfile::TempDir) -> (SystemdUserServiceManager, PathBuf) {
    let unit_dir = tmp.path().join("systemd/user");
    let log = tmp.path().join("systemctl.log");
    let fake = fake_systemctl(tmp.path(), &log);
    (
        SystemdUserServiceManager::new(unit_dir).with_systemctl_binary(fake),
        log,
    )
}

#[test]
fn write_read_and_remove_the_unit_file() {
    let tmp = tempfile::tempdir().unwrap();
    let (m, _log) = manager(&tmp);
    assert_eq!(m.unit_status().unwrap(), UnitStatus::Absent);
    m.write_unit(&ServiceUnitSpec {
        contents: "hello".into(),
    })
    .unwrap();
    assert_eq!(
        m.unit_status().unwrap(),
        UnitStatus::Present {
            contents: "hello".into()
        }
    );
    assert!(m.unit_path().ends_with(UNIT_NAME));
    assert!(m.remove_unit().unwrap());
    assert!(!m.remove_unit().unwrap());
    assert_eq!(m.unit_status().unwrap(), UnitStatus::Absent);
}

#[test]
fn systemctl_lifecycle_calls_reach_the_binary() {
    let tmp = tempfile::tempdir().unwrap();
    let (m, log) = manager(&tmp);
    m.daemon_reload().unwrap();
    m.enable_now().unwrap();
    assert!(m.disable_now().unwrap());
    let logged = std::fs::read_to_string(&log).unwrap();
    assert!(logged.contains("--user daemon-reload"), "{logged}");
    assert!(
        logged.contains(&format!("--user enable --now {UNIT_NAME}")),
        "{logged}"
    );
    assert!(
        logged.contains(&format!("--user disable --now {UNIT_NAME}")),
        "{logged}"
    );
}
