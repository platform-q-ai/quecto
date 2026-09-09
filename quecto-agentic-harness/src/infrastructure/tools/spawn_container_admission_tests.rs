//! #1679 P3: admission-enabled parents hand script-managed runtimes the
//! authority client directory and refuse launches whose scripts do not report
//! the shared-directory capability. Disabled parents are unchanged.
use super::*;
use std::path::PathBuf;

fn create_json(extra: &str) -> Vec<u8> {
    format!(
        r#"{{"environment_id":"env-1","workspace_path":"/tmp/ws","metadata":{{}},"socket_path":"/tmp/x.sock"{extra}}}"#
    )
    .into_bytes()
}

#[test]
fn disabled_parent_ignores_and_does_not_require_the_capability() {
    assert!(parse_create_result(&create_json(""), None).is_ok());
    assert!(
        parse_create_result(
            &create_json(r#","admission_capability":"shared-directory-v1""#),
            None
        )
        .is_ok()
    );
    assert!(parse_exec_result(br#"{"metadata":{},"socket_path":"/tmp/s"}"#, None).is_ok());
}

#[test]
fn enabled_parent_accepts_only_the_shared_directory_capability() {
    let dir = PathBuf::from("/tmp/authority/client");
    assert!(
        parse_create_result(
            &create_json(r#","admission_capability":"shared-directory-v1""#),
            Some(&dir)
        )
        .is_ok()
    );
    let missing = parse_create_result(&create_json(""), Some(&dir)).unwrap_err();
    assert!(
        missing.to_string().contains("shared-directory-v1")
            && missing.to_string().contains("never bypass"),
        "{missing}"
    );
    let wrong = parse_create_result(
        &create_json(r#","admission_capability":"reverse-stdio-v1""#),
        Some(&dir),
    )
    .unwrap_err();
    assert!(wrong.to_string().contains("reverse-stdio-v1"), "{wrong}");
    let exec_missing =
        parse_exec_result(br#"{"metadata":{},"socket_path":"/tmp/s"}"#, Some(&dir)).unwrap_err();
    assert!(exec_missing.to_string().contains("exec"), "{exec_missing}");
    assert!(
        parse_exec_result(
            br#"{"metadata":{},"socket_path":"/tmp/s","admission_capability":"shared-directory-v1"}"#,
            Some(&dir)
        )
        .is_ok()
    );
}

#[test]
fn admission_env_is_set_only_for_enabled_parents() {
    let dir = PathBuf::from("/tmp/authority/client");
    let mut enabled = tokio::process::Command::new("true");
    apply_admission_env(&mut enabled, Some(&dir));
    let envs: Vec<_> = enabled.as_std().get_envs().collect();
    assert_eq!(
        envs,
        vec![(
            std::ffi::OsStr::new("QUECTO_ADMISSION_DIR"),
            Some(std::ffi::OsStr::new("/tmp/authority/client"))
        )]
    );
    let mut disabled = tokio::process::Command::new("true");
    apply_admission_env(&mut disabled, None);
    assert_eq!(disabled.as_std().get_envs().count(), 0);
}
