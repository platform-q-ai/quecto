use super::*;

fn ip(text: &str) -> IpAddr {
    text.parse().unwrap()
}

/// Review round 2: loopback (127.0.0.0/8 and ::1) is one source; every
/// other address is its own.
#[test]
fn loopback_is_one_source_and_other_addresses_their_own() {
    let local = source_of(ip("127.0.0.1"));
    assert_eq!(source_of(ip("127.5.6.7")), local);
    assert_eq!(source_of(ip("::1")), local);
    assert_eq!(source_of(ip("100.64.1.2")), ip("100.64.1.2"));
    assert_ne!(source_of(ip("100.64.1.2")), source_of(ip("100.64.1.3")));
    assert_eq!(source_of(ip("fd7a:115c:a1e0::1")), ip("fd7a:115c:a1e0::1"));
}

fn told(unproven: &Unproven) -> bool {
    let notified = unproven.evicted.notified();
    futures_like_now(notified)
}

/// Whether `future` is ready at once.
fn futures_like_now(future: impl std::future::Future<Output = ()>) -> bool {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime.block_on(async {
        tokio::time::timeout(std::time::Duration::from_millis(10), future)
            .await
            .is_ok()
    })
}

/// Review round 3: one more unauthenticated connection than a source may
/// hold evicts its oldest; one that authenticates (is dropped) frees its
/// place; other sources are untouched.
#[test]
fn one_more_evicts_the_oldest_of_its_source() {
    let sources = Sources::default();
    let local = ip("127.0.0.1");
    let first = sources.enter(local, 2);
    let second = sources.enter(local, 2);
    let other = sources.enter(ip("100.64.1.2"), 2);
    let third = sources.enter(local, 2);
    assert!(told(&first), "the oldest is told to close");
    assert!(!told(&second) && !told(&third) && !told(&other));
    drop(first);
    drop(second);
    let fourth = sources.enter(local, 2);
    assert!(!told(&third) && !told(&fourth), "a place given up is free");
}
