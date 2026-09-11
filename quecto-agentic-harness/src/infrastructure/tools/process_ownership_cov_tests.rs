//! Coverage for the lease state machine edges (#1925): poisoned-lock recovery
//! on every accessor, retirement rules per state, and the runtime refusal.

use super::*;

fn poison(ownership: &ProcessOwnership) {
    let shared = ownership.clone();
    let _ = std::thread::spawn(move || {
        let _guard = shared.0.lock().unwrap();
        panic!("poison lease for coverage");
    })
    .join();
    assert!(ownership.0.lock().is_err(), "lease should be poisoned");
}

#[test]
fn retire_reported_only_retires_reported_leases() {
    let launched = ProcessOwnership::launched_for_test();
    launched.retire_reported();
    assert_eq!(
        launched.lease(),
        Lease::Launched,
        "reaper owns launched leases"
    );
    assert!(launched.is_launched());

    let reported = ProcessOwnership::reported(true);
    assert!(reported.is_same_namespace());
    reported.retire_reported();
    assert_eq!(reported.lease(), Lease::Unowned);
    assert!(!reported.is_same_namespace());

    let foreign = ProcessOwnership::reported(false);
    assert!(!foreign.is_same_namespace());
    foreign.retire_reported();
    assert_eq!(foreign.lease(), Lease::Unowned);

    let unowned = ProcessOwnership::unowned();
    unowned.retire_reported();
    assert_eq!(unowned.lease(), Lease::Unowned);
    assert!(!unowned.is_launched());
}

#[test]
fn every_accessor_recovers_from_a_poisoned_lease() {
    let ownership = ProcessOwnership::reported(true);
    poison(&ownership);
    assert_eq!(
        ownership.lease(),
        Lease::Reported {
            same_namespace: true
        }
    );
    assert!(ownership.is_owned());
    assert!(!ownership.is_launched());
    assert!(ownership.is_same_namespace());
    // An unowned dispatch on a poisoned lease still refuses.
    ownership.retire_reported();
    assert!(!ownership.signal(0, super::super::process_tree::ProcessOwner::DirectPid));
    assert!(!ownership.dispatch(|| panic!("unowned lease must not dispatch")));
}

#[tokio::test]
async fn wait_recovers_from_a_poisoned_lease_and_retires_it() {
    let mut child = tokio::process::Command::new("true").spawn().unwrap();
    let ownership = ProcessOwnership::launched(&child);
    poison(&ownership);
    let status = ownership.wait(&mut child).await.unwrap();
    assert!(status.success());
    assert_eq!(
        ownership.lease(),
        Lease::Unowned,
        "reap retires every clone"
    );
}
