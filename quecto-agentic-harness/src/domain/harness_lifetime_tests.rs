use super::{HarnessLifetime, HarnessLifetimeError};

#[test]
fn top_level_default_exits_when_last_client_disconnects() {
    let lifetime = HarnessLifetime::resolve(false, false).unwrap();
    assert_eq!(lifetime, HarnessLifetime::UntilLastClientDisconnects);
    assert!(lifetime.exits_when_last_client_disconnects());
    assert!(!lifetime.is_launch_bound());
}

#[test]
fn top_level_persist_survives_client_churn() {
    let lifetime = HarnessLifetime::resolve(true, false).unwrap();
    assert_eq!(lifetime, HarnessLifetime::Persistent);
    assert!(!lifetime.exits_when_last_client_disconnects());
    assert!(!lifetime.is_launch_bound());
}

#[test]
fn launched_child_is_launch_bound_and_ignores_client_churn() {
    let lifetime = HarnessLifetime::resolve(false, true).unwrap();
    assert_eq!(lifetime, HarnessLifetime::LaunchBound);
    assert!(!lifetime.exits_when_last_client_disconnects());
    assert!(lifetime.is_launch_bound());
}

/// #1937: a launcher-created child cannot outlive its launcher, so a
/// persistent launched child is refused rather than silently narrowed.
#[test]
fn launched_child_refuses_persist() {
    let err = HarnessLifetime::resolve(true, true).unwrap_err();
    assert_eq!(err, HarnessLifetimeError::LaunchedChildCannotPersist);
    assert!(err.to_string().contains("--persist is refused"));
}
