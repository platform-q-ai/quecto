use super::*;
use crate::domain::parent_control::CAPABILITY_HEX_LEN;

#[test]
fn minted_credentials_are_random_and_generation_scoped() {
    let a = mint_credential();
    let b = mint_credential();
    assert_ne!(a.capability, b.capability);
    assert!(b.generation > a.generation);
    assert_eq!(a.capability.expose().len(), CAPABILITY_HEX_LEN);
}

#[test]
fn sidecar_round_trips_privately_and_is_single_use() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("quecto-parent-control-test");
    let credential = mint_credential();
    write_sidecar(&path, &credential).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    // Exclusive creation: a second write to the same path is refused.
    assert_eq!(
        write_sidecar(&path, &credential).unwrap_err().kind(),
        std::io::ErrorKind::AlreadyExists
    );
    let taken = take_sidecar(&path).unwrap();
    assert_eq!(taken, credential);
    assert!(!path.exists(), "the sidecar is removed once read");
    let missing = take_sidecar(&path).unwrap_err();
    assert!(missing.contains("unreadable"), "{missing}");
}

#[test]
fn sidecar_rejects_malformed_unsupported_and_bad_capabilities() {
    let dir = tempfile::tempdir().unwrap();
    let cases = [
        ("not json", "malformed"),
        (
            r#"{"format":2,"generation":1,"capability":"aa"}"#,
            "format 2 unsupported",
        ),
        (
            r#"{"format":1,"generation":1,"capability":"zz"}"#,
            "must be 64 hex",
        ),
    ];
    for (body, expected) in cases {
        let path = dir.path().join("sidecar");
        std::fs::write(&path, body).unwrap();
        let error = take_sidecar(&path).unwrap_err();
        assert!(error.contains(expected), "{error}");
        assert!(!path.exists(), "a rejected sidecar is removed too");
    }
}

#[test]
fn presentation_json_carries_type_generation_and_material() {
    let credential = mint_credential();
    let line = presentation_json(&credential);
    let value: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(value["type"], BIND_PARENT_CONTROL);
    assert_eq!(value["generation"], credential.generation.get());
    assert_eq!(value["capability"], credential.capability.expose());
    assert!(!line.contains('\n'));
}
