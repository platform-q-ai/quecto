use super::*;

#[test]
fn raised_by_a_client_and_withdrawn_only_by_that_client() {
    let flag = OwnerExitFlag::new();
    assert!(!flag.announced());
    flag.announce(7);
    assert!(flag.announced());
    flag.withdraw(3);
    assert!(flag.announced(), "another client's close leaves it raised");
    flag.withdraw(7);
    assert!(!flag.announced());
    flag.withdraw(7);
    assert!(!flag.announced());
}

#[test]
fn a_later_announcer_takes_the_announcement_over() {
    let flag = OwnerExitFlag::new();
    flag.announce(1);
    flag.announce(2);
    flag.withdraw(1);
    assert!(flag.announced(), "client 2 still holds it");
    flag.withdraw(2);
    assert!(!flag.announced());
}
