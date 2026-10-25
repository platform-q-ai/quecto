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
    let notified = unproven.signals.evicted.notified();
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

/// Review round 4: eviction takes the oldest connection that has sent
/// nothing, sparing an older one that is sending, slowly.
#[test]
fn eviction_spares_a_connection_that_is_sending() {
    let sources = Sources::default();
    let local = ip("127.0.0.1");
    let sending = sources.enter(local, 2);
    sending.signals.heard.store(true, Ordering::Relaxed);
    let silent = sources.enter(local, 2);
    let newcomer = sources.enter(local, 2);
    assert!(told(&silent), "the oldest silent one is evicted");
    assert!(!told(&sending) && !told(&newcomer));
    assert!(sending.loopback() && !sources.enter(ip("100.64.1.2"), 2).loopback());
}

/// Review round 4: one principal holds at most `per` blocking answers,
/// and every TCP caller together at most `all`; a permit given back frees
/// its place.
#[test]
fn answers_are_bounded_per_principal_and_in_all() {
    let answers = Answers::new(3, 2);
    let first = answers.admit("laptop").unwrap();
    let _second = answers.admit("laptop").unwrap();
    assert!(answers.admit("laptop").is_none(), "laptop's share is full");
    let _phone = answers.admit("phone").unwrap();
    assert!(
        answers.admit("tablet").is_none(),
        "every caller's share is full"
    );
    drop(first);
    assert!(answers.admit("laptop").is_some());
}

/// #2727: each principal's share is its own, `per` at once, and a place
/// given back is taken again.
#[test]
fn shares_are_bounded_per_principal_and_given_back() {
    let shares = Shares::new(2);
    let first = shares.take("laptop").unwrap();
    let _second = shares.take("laptop").unwrap();
    assert!(shares.take("laptop").is_none(), "laptop's share is full");
    assert!(shares.take("phone").is_some(), "phone's is its own");
    drop(first);
    assert!(shares.take("laptop").is_some(), "a place given back");
}

/// #2727: a share holds at least one, or no principal could ever be served.
#[test]
#[should_panic(expected = "at least one")]
fn an_empty_share_is_refused() {
    let _ = Shares::new(0);
}
