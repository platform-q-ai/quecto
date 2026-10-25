use super::*;

/// A descriptor inherited across a fork shares the lock's open file
/// description: kept open deterministically, rather than relying on a
/// concurrent spawn to hit the fork-to-program window (#1492).
#[test]
fn the_lock_is_released_on_drop_while_an_inherited_descriptor_stays_open() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    private_dir(&state).unwrap();
    let lock = locked(&state, "held").unwrap();
    let inherited = lock.inherited_descriptor().unwrap();
    let error = locked(&state, "held").unwrap_err();
    assert!(error.ends_with(": held"), "{error}");
    drop(lock);
    let again = locked(&state, "held");
    assert!(
        again.is_ok(),
        "released though inherited: {:?}",
        again.err()
    );
    drop(inherited);
}
