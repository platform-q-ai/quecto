use super::*;
use crate::infrastructure::processes::parent_control::{mint_credential, write_sidecar};

#[test]
fn top_level_harness_has_no_binding() {
    let mut stderr = String::new();
    assert!(matches!(consume(None, true, &mut stderr), Some(None)));
    assert!(stderr.is_empty());
}

#[test]
fn a_readable_sidecar_becomes_a_launched_binding_and_is_consumed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sidecar");
    write_sidecar(&path, &mint_credential()).unwrap();
    let mut stderr = String::new();
    let launch = consume(Some(&path), true, &mut stderr).unwrap().unwrap();
    assert!(launch.binding.is_launched_child());
    assert!(matches!(launch.bind_deadline, BindDeadline::After(d) if d == DEFAULT_BIND_DEADLINE));
    assert!(!path.exists());
}

#[test]
fn bind_deadline_override_must_be_a_positive_millisecond_count() {
    assert_eq!(bind_deadline(None), DEFAULT_BIND_DEADLINE);
    assert_eq!(bind_deadline(Some("0")), DEFAULT_BIND_DEADLINE);
    assert_eq!(bind_deadline(Some("soon")), DEFAULT_BIND_DEADLINE);
    assert_eq!(
        bind_deadline(Some(" 250 ")),
        std::time::Duration::from_millis(250)
    );
}

#[test]
fn a_missing_sidecar_fails_startup_closed() {
    let mut stderr = String::new();
    let missing = std::path::Path::new("/nonexistent/quecto-parent-control");
    assert!(consume(Some(missing), true, &mut stderr).is_none());
    assert!(stderr.contains("agent: parent control:"), "{stderr}");
}

#[test]
fn a_launched_child_without_the_composed_graph_fails_closed_and_consumes_the_sidecar() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sidecar");
    write_sidecar(&path, &mint_credential()).unwrap();
    let mut stderr = String::new();
    assert!(consume(Some(&path), false, &mut stderr).is_none());
    assert!(stderr.contains("composed teardown graph"), "{stderr}");
    assert!(!path.exists(), "the material never lingers");
}
