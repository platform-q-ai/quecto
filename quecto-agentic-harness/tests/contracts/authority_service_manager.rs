//! Contract for the `AuthorityServiceManager` port (#2024 S3): the unit file
//! is written, read back and removed idempotently, and each lifecycle call
//! reaches `systemctl --user`. A fake `systemctl` (pointed at by the manager,
//! so the whole test process's PATH is untouched) records every invocation.
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use quecto::application::admission::ports::{AuthorityServiceManager, ServiceUnitSpec, UnitStatus};
use quecto::infrastructure::processes::local::SystemdUserServiceManager;

fn fake_systemctl(dir: &Path, log: &Path) -> PathBuf {
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

fn under_test(tmp: &tempfile::TempDir) -> (SystemdUserServiceManager, PathBuf) {
    let log = tmp.path().join("systemctl.log");
    let fake = fake_systemctl(tmp.path(), &log);
    (
        SystemdUserServiceManager::new(tmp.path().join("systemd/user")).with_systemctl_binary(fake),
        log,
    )
}

#[test]
fn the_unit_file_round_trips_and_removal_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let (m, _log) = under_test(&tmp);
    assert_eq!(m.unit_status().unwrap(), UnitStatus::Absent);
    assert!(m.unit_name().ends_with(".service"));
    assert!(m.unit_path().ends_with(m.unit_name()));

    m.write_unit(&ServiceUnitSpec {
        contents: "[Service]\nExecStart=/x\n".into(),
    })
    .unwrap();
    assert!(matches!(
        m.unit_status().unwrap(),
        UnitStatus::Present { .. }
    ));

    assert!(
        m.remove_unit().unwrap(),
        "first removal reports it removed one"
    );
    assert!(!m.remove_unit().unwrap(), "a second removal is a no-op");
    assert_eq!(m.unit_status().unwrap(), UnitStatus::Absent);
}

#[test]
fn lifecycle_calls_reach_systemctl_user() {
    let tmp = tempfile::tempdir().unwrap();
    let (m, log) = under_test(&tmp);
    m.daemon_reload().unwrap();
    m.enable_now().unwrap();
    m.restart().unwrap();
    assert!(m.disable_now().unwrap());
    let logged = std::fs::read_to_string(&log).unwrap();
    assert!(logged.contains("--user daemon-reload"), "{logged}");
    assert!(logged.contains("--user enable --now"), "{logged}");
    assert!(
        logged.contains("--user restart quecto-admission-broker.service"),
        "{logged}"
    );
    assert!(logged.contains("--user disable --now"), "{logged}");
}

#[test]
fn failed_systemctl_user_commands_report_stderr_and_preserve_unit() {
    let tmp = tempfile::tempdir().unwrap();
    let (manager, log) = under_test(&tmp);
    let script = tmp.path().join("systemctl");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nprintf 'service unavailable\\n' >&2\nexit 7\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    manager
        .write_unit(&ServiceUnitSpec {
            contents: "[Service]\nExecStart=/x\n".into(),
        })
        .unwrap();
    for (args, result) in [
        ("--user daemon-reload", manager.daemon_reload()),
        (
            "--user enable --now quecto-admission-broker.service",
            manager.enable_now(),
        ),
        (
            "--user restart quecto-admission-broker.service",
            manager.restart(),
        ),
    ] {
        let error = result.expect_err("nonzero exit must be reported");
        assert!(error.contains(args), "{error}");
        assert!(error.contains("service unavailable"), "{error}");
    }
    let error = manager
        .disable_now()
        .expect_err("non-idempotent failure must propagate");
    assert!(
        error.contains("--user disable --now quecto-admission-broker.service"),
        "{error}"
    );
    assert!(error.contains("service unavailable"), "{error}");
    assert!(matches!(
        manager.unit_status().unwrap(),
        UnitStatus::Present { .. }
    ));
    assert_eq!(
        std::fs::read_to_string(log)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        [
            "--user daemon-reload",
            "--user enable --now quecto-admission-broker.service",
            "--user restart quecto-admission-broker.service",
            "--user disable --now quecto-admission-broker.service",
        ]
    );
}
